/*
 * Copyright [2022] [DMetaSoul Team]
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::lakesoul_io_config::{create_session_context, LakeSoulIOConfig};
use crate::lakesoul_reader::ArrowResult;
use arrow::compute::SortOptions;
use arrow::record_batch::RecordBatch;
use arrow_schema::SchemaRef;
use async_trait::async_trait;
use atomic_refcell::AtomicRefCell;
use datafusion::datasource::object_store::ObjectStoreUrl;
use datafusion::error::Result;
use datafusion::execution::context::TaskContext;
use datafusion::physical_expr::expressions::Column;
use datafusion::physical_expr::PhysicalSortExpr;
use datafusion::physical_plan::sorts::sort::SortExec;
use datafusion::physical_plan::stream::RecordBatchReceiverStream;
use datafusion::physical_plan::{ExecutionPlan, Partitioning, SendableRecordBatchStream, Statistics};
use datafusion::prelude::SessionContext;
use datafusion_common::DataFusionError;
use datafusion_common::DataFusionError::Internal;
use object_store::path::Path;
use object_store::MultipartId;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::any::Any;
use std::collections::VecDeque;
use std::io::ErrorKind::ResourceBusy;
use std::io::Write;
use std::sync::Arc;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use url::{ParseError, Url};

#[async_trait]
pub trait AsyncWriter {
    async fn write_record_batch(&mut self, batch: RecordBatch) -> Result<()>;

    async fn flush_and_close(self: Box<Self>) -> Result<()>;
}

/// An async writer using object_store's multi-part upload feature for cloud storage.
/// This writer uses a `VecDeque<u8>` as `std::io::Write` for arrow-rs's ArrowWriter.
/// Everytime when a new RowGroup is flushed, the length of the VecDeque would grow.
/// At this time, we pass the VecDeque as `bytes::Buf` to `AsyncWriteExt::write_buf` provided
/// by object_store, which would drain and copy the content of the VecDeque so that we could reuse it.
/// The CloudMultiPartUpload itself would try to concurrently upload parts, and
/// all parts will be committed to cloud storage by shutdown the AsyncWrite.
pub struct MultiPartAsyncWriter {
    in_mem_buf: InMemBuf,
    sess_ctx: SessionContext,
    schema: SchemaRef,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
    multi_part_id: MultipartId,
    arrow_writer: ArrowWriter<InMemBuf>,
    config: LakeSoulIOConfig,
}

/// Wrap the above async writer with a SortExec to
/// sort the batches before write to async writer
pub struct SortAsyncWriter {
    sorter_sender: Sender<ArrowResult<RecordBatch>>,
    sort_exec: Arc<SortExec>,
    join_handle: JoinHandle<Result<()>>,
}

/// A VecDeque which is both std::io::Write and bytes::Buf
#[derive(Clone)]
struct InMemBuf(Arc<AtomicRefCell<VecDeque<u8>>>);

impl Write for InMemBuf {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut v = self
            .0
            .try_borrow_mut()
            .map_err(|_| std::io::Error::from(ResourceBusy))?;
        v.extend(buf);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        let mut v = self
            .0
            .try_borrow_mut()
            .map_err(|_| std::io::Error::from(ResourceBusy))?;
        v.extend(buf);
        Ok(())
    }
}

#[derive(Debug)]
pub struct ReceiverStreamExec {
    stream: AtomicRefCell<Option<tokio::sync::mpsc::Receiver<ArrowResult<RecordBatch>>>>,
    join_handle: AtomicRefCell<Option<JoinHandle<()>>>,
    schema: SchemaRef,
}

impl ReceiverStreamExec {
    pub fn new(
        receiver: tokio::sync::mpsc::Receiver<ArrowResult<RecordBatch>>,
        join_handle: JoinHandle<()>,
        schema: SchemaRef,
    ) -> Self {
        Self {
            stream: AtomicRefCell::new(Some(receiver)),
            join_handle: AtomicRefCell::new(Some(join_handle)),
            schema,
        }
    }
}

impl ExecutionPlan for ReceiverStreamExec {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn output_partitioning(&self) -> Partitioning {
        Partitioning::UnknownPartitioning(1)
    }

    fn output_ordering(&self) -> Option<&[PhysicalSortExpr]> {
        None
    }

    fn children(&self) -> Vec<Arc<dyn ExecutionPlan>> {
        unimplemented!()
    }

    fn with_new_children(self: Arc<Self>, children: Vec<Arc<dyn ExecutionPlan>>) -> Result<Arc<dyn ExecutionPlan>> {
        unimplemented!()
    }

    fn execute(&self, partition: usize, context: Arc<TaskContext>) -> Result<SendableRecordBatchStream> {
        let receiver_stream = self.stream.borrow_mut().take().unwrap();
        let join_handle = self.join_handle.borrow_mut().take().unwrap();
        let stream = RecordBatchReceiverStream::create(&self.schema, receiver_stream, join_handle);
        Ok(stream)
    }

    fn statistics(&self) -> Statistics {
        Statistics::default()
    }
}

impl MultiPartAsyncWriter {
    pub async fn try_new(mut config: LakeSoulIOConfig) -> Result<Self> {
        if config.files.len() != 1 {
            return Err(Internal("wrong number of file names provided for writer".to_string()));
        }
        let sess_ctx = create_session_context(&mut config)?;
        let file_name = &config.files[0];

        // parse file name. Url::parse requires file:// scheme for local files, otherwise
        // RelativeUrlWithoutBase would be throw, in this case we directly return local object store
        let (object_store, path) = match Url::parse(file_name.as_str()) {
            Ok(url) => Ok((
                sess_ctx
                    .runtime_env()
                    .object_store(ObjectStoreUrl::parse(&url[..url::Position::BeforePath])?)?,
                Path::from(url.path()),
            )),
            Err(ParseError::RelativeUrlWithoutBase) => Ok((
                sess_ctx
                    .runtime_env()
                    .object_store(ObjectStoreUrl::local_filesystem())?,
                Path::from(file_name.as_str()),
            )),
            Err(e) => Err(DataFusionError::External(Box::new(e))),
        }?;

        let (multipart_id, async_writer) = object_store.put_multipart(&path).await?;
        let in_mem_buf = InMemBuf(Arc::new(AtomicRefCell::new(VecDeque::<u8>::with_capacity(
            16 * 1024 * 1024,
        ))));
        let schema: SchemaRef =
            Arc::new(serde_json::from_str(&config.schema_json).map_err(|e| DataFusionError::External(Box::new(e)))?);

        let arrow_writer = ArrowWriter::try_new(
            in_mem_buf.clone(),
            schema.clone(),
            Some(
                WriterProperties::builder()
                    .set_max_row_group_size(config.max_row_group_size)
                    .set_write_batch_size(config.batch_size)
                    .set_compression(Compression::SNAPPY)
                    .build(),
            ),
        )?;

        Ok(MultiPartAsyncWriter {
            in_mem_buf: in_mem_buf.clone(),
            sess_ctx,
            schema: schema.clone(),
            writer: async_writer,
            multi_part_id: multipart_id,
            arrow_writer,
            config,
        })
    }

    async fn write_batch(
        batch: RecordBatch,
        arrow_writer: &mut ArrowWriter<InMemBuf>,
        in_mem_buf: &mut InMemBuf,
        writer: &mut Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<()> {
        arrow_writer.write(&batch)?;
        let mut v = in_mem_buf
            .0
            .try_borrow_mut()
            .map_err(|e| Internal(format!("{:?}", e)))?;
        if v.len() > 0 {
            MultiPartAsyncWriter::write_part(writer, &mut *v).await
        } else {
            Ok(())
        }
    }

    pub async fn write_part(
        writer: &mut Box<dyn AsyncWrite + Unpin + Send>,
        in_mem_buf: &mut VecDeque<u8>,
    ) -> Result<()> {
        writer.write_all_buf(in_mem_buf).await?;
        Ok(())
    }
}

#[async_trait]
impl AsyncWriter for MultiPartAsyncWriter {
    async fn write_record_batch(&mut self, batch: RecordBatch) -> Result<()> {
        MultiPartAsyncWriter::write_batch(batch, &mut self.arrow_writer, &mut self.in_mem_buf, &mut self.writer).await
    }

    async fn flush_and_close(self: Box<Self>) -> Result<()> {
        // close arrow writer to flush remaining rows
        let mut this = *self;
        let arrow_writer = this.arrow_writer;
        arrow_writer.close()?;
        let mut v = this
            .in_mem_buf
            .0
            .try_borrow_mut()
            .map_err(|e| Internal(format!("{:?}", e)))?;
        if v.len() > 0 {
            MultiPartAsyncWriter::write_part(&mut this.writer, &mut *v).await?;
        }
        // shutdown multi part async writer to complete the upload
        this.writer.shutdown().await?;
        Ok(())
    }
}

impl SortAsyncWriter {
    pub fn try_new(
        async_writer: MultiPartAsyncWriter,
        config: LakeSoulIOConfig,
        schema: SchemaRef,
        runtime: Arc<Runtime>,
    ) -> Result<Self> {
        let _ = runtime.enter();
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        let recv_exec = ReceiverStreamExec::new(rx, tokio::task::spawn(async move {}), schema.clone());

        let sort_exprs: Vec<PhysicalSortExpr> = config
            .primary_keys
            .iter()
            .map(|pk| {
                let col = Column::new_with_schema(pk.as_str(), &*schema)?;
                Ok(PhysicalSortExpr {
                    expr: Arc::new(col),
                    options: SortOptions::default(),
                })
            })
            .collect::<Result<Vec<PhysicalSortExpr>>>()?;
        let sort_exec = Arc::new(SortExec::try_new(sort_exprs, Arc::new(recv_exec), None)?);
        let mut sorted_stream = sort_exec.execute(0, async_writer.sess_ctx.task_ctx())?;

        let mut async_writer = Box::new(async_writer);
        let join_handle = tokio::task::spawn(async move {
            while let Some(batch) = sorted_stream.next().await {
                let batch = batch?;
                async_writer.write_record_batch(batch).await?;
            }
            async_writer.flush_and_close().await?;
            Ok(())
        });

        Ok(SortAsyncWriter {
            sorter_sender: tx,
            sort_exec,
            join_handle,
        })
    }
}

#[async_trait]
impl AsyncWriter for SortAsyncWriter {
    async fn write_record_batch(&mut self, batch: RecordBatch) -> Result<()> {
        self.sorter_sender
            .send(Ok(batch))
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))
    }

    async fn flush_and_close(self: Box<Self>) -> Result<()> {
        let sender = self.sorter_sender;
        drop(sender);
        self.join_handle
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?
    }
}

pub struct SyncSendableMutableLakeSoulWriter {
    inner: Arc<Mutex<Box<dyn AsyncWriter>>>,
    runtime: Arc<Runtime>,
    schema: SchemaRef,
}

impl SyncSendableMutableLakeSoulWriter {
    pub fn new(config: LakeSoulIOConfig, runtime: Runtime) -> Result<Self> {
        let runtime = Arc::new(runtime);
        runtime.clone().block_on(async move {
            let writer = MultiPartAsyncWriter::try_new(config.clone()).await?;
            let schema = writer.schema.clone();
            let writer: Box<dyn AsyncWriter> = if !config.primary_keys.is_empty() {
                Box::new(SortAsyncWriter::try_new(
                    writer,
                    config,
                    schema.clone(),
                    runtime.clone(),
                )?)
            } else {
                Box::new(writer)
            };
            Ok(SyncSendableMutableLakeSoulWriter {
                inner: Arc::new(Mutex::new(writer)),
                runtime,
                schema,
            })
        })
    }

    // blocking method for writer record batch.
    // since the underlying multipart upload would accumulate buffers
    // and upload concurrently in background, we only need blocking method here
    // for ffi callers
    pub fn write_batch(&self, record_batch: RecordBatch) -> Result<()> {
        let inner_writer = self.inner.clone();
        let runtime = self.runtime.clone();
        runtime.block_on(async move {
            let mut writer = inner_writer.lock().await;
            writer.write_record_batch(record_batch).await
        })
    }

    pub fn flush_and_close(self) -> Result<()> {
        let inner_writer = match Arc::try_unwrap(self.inner) {
            Ok(inner) => inner,
            Err(_) => return Err(Internal("Cannot get ownership of inner writer".to_string())),
        };
        let runtime = self.runtime;
        runtime.block_on(async move {
            let writer = inner_writer.into_inner();
            writer.flush_and_close().await
        })
    }

    pub fn get_schema(&self) -> SchemaRef {
        self.schema.clone()
    }
}

#[cfg(test)]
mod tests {
    use crate::lakesoul_io_config::LakeSoulIOConfigBuilder;
    use crate::lakesoul_reader::LakeSoulReader;
    use crate::lakesoul_writer::{AsyncWriter, MultiPartAsyncWriter, SortAsyncWriter};
    use arrow::array::{ArrayRef, Int64Array};
    use arrow::record_batch::RecordBatch;
    use arrow_schema::Schema;
    use datafusion::error::Result;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReader;
    use std::borrow::Borrow;
    use std::fs::File;
    use std::sync::Arc;
    use tokio::runtime::Builder;

    #[test]
    fn test_parquet_async_write() -> Result<()> {
        let runtime = Arc::new(Builder::new_multi_thread().enable_all().build().unwrap());
        runtime.clone().block_on(async move {
            let col = Arc::new(Int64Array::from_iter_values([3, 2, 1])) as ArrayRef;
            let to_write = RecordBatch::try_from_iter([("col", col)])?;
            let temp_dir = tempfile::tempdir()?;
            let path = temp_dir
                .into_path()
                .join("test.parquet")
                .into_os_string()
                .into_string()
                .unwrap();
            let writer_conf = LakeSoulIOConfigBuilder::new()
                .with_files(vec![path.clone()])
                .with_thread_num(2)
                .with_batch_size(256)
                .with_max_row_group_size(2)
                .with_schema_json(serde_json::to_string::<Schema>(to_write.schema().borrow()).unwrap())
                .build();
            let mut async_writer = MultiPartAsyncWriter::try_new(writer_conf).await?;
            async_writer.write_record_batch(to_write.clone()).await?;
            Box::new(async_writer).flush_and_close().await?;

            let file = File::open(path.clone())?;
            let mut record_batch_reader = ParquetRecordBatchReader::try_new(file, 1024).unwrap();

            let actual_batch = record_batch_reader
                .next()
                .expect("No batch found")
                .expect("Unable to get batch");

            assert_eq!(to_write.schema(), actual_batch.schema());
            assert_eq!(to_write.num_columns(), actual_batch.num_columns());
            assert_eq!(to_write.num_rows(), actual_batch.num_rows());
            for i in 0..to_write.num_columns() {
                let expected_data = to_write.column(i).data();
                let actual_data = actual_batch.column(i).data();

                assert_eq!(expected_data, actual_data);
            }

            let writer_conf = LakeSoulIOConfigBuilder::new()
                .with_files(vec![path.clone()])
                .with_thread_num(2)
                .with_batch_size(256)
                .with_max_row_group_size(2)
                .with_schema_json(serde_json::to_string::<Schema>(to_write.schema().borrow()).unwrap())
                .with_primary_keys(vec!["col".to_string()])
                .build();

            let async_writer = MultiPartAsyncWriter::try_new(writer_conf.clone()).await?;
            let schema = async_writer.schema.clone();
            let mut async_writer = SortAsyncWriter::try_new(async_writer, writer_conf, schema, runtime.clone())?;
            async_writer.write_record_batch(to_write.clone()).await?;
            Box::new(async_writer).flush_and_close().await?;

            let file = File::open(path)?;
            let mut record_batch_reader = ParquetRecordBatchReader::try_new(file, 1024).unwrap();

            let actual_batch = record_batch_reader
                .next()
                .expect("No batch found")
                .expect("Unable to get batch");

            let col = Arc::new(Int64Array::from_iter_values([1, 2, 3])) as ArrayRef;
            let to_read = RecordBatch::try_from_iter([("col", col)])?;
            assert_eq!(to_read.schema(), actual_batch.schema());
            assert_eq!(to_read.num_columns(), actual_batch.num_columns());
            assert_eq!(to_read.num_rows(), actual_batch.num_rows());
            for i in 0..to_read.num_columns() {
                let expected_data = to_read.column(i).data();
                let actual_data = actual_batch.column(i).data();

                assert_eq!(expected_data, actual_data);
            }
            Ok(())
        })
    }

    #[tokio::test]
    async fn test_s3_read_write() -> Result<()> {
        let common_conf_builder = LakeSoulIOConfigBuilder::new()
            .with_thread_num(2)
            .with_batch_size(8192)
            .with_max_row_group_size(250000)
            .with_object_store_option("fs.s3a.access.key".to_string(), "minioadmin1".to_string())
            .with_object_store_option("fs.s3a.access.secret".to_string(), "minioadmin1".to_string())
            .with_object_store_option("fs.s3a.endpoint".to_string(), "http://localhost:9000".to_string());

        let read_conf = common_conf_builder
            .clone()
            .with_files(vec![
                "s3://lakesoul-test-bucket/data/native-io-test/large_file.parquet".to_string()
            ])
            .build();
        let mut reader = LakeSoulReader::new(read_conf)?;
        reader.start().await?;

        let schema = reader.schema.clone().unwrap();

        let write_conf = common_conf_builder
            .clone()
            .with_files(vec![
                "s3://lakesoul-test-bucket/data/native-io-test/large_file_written.parquet".to_string(),
            ])
            .with_schema_json(serde_json::to_string::<Schema>(schema.borrow()).unwrap())
            .build();
        let mut async_writer = MultiPartAsyncWriter::try_new(write_conf).await?;

        while let Some(rb) = reader.next_rb().await {
            let rb = rb?;
            async_writer.write_record_batch(rb).await?;
        }

        Box::new(async_writer).flush_and_close().await?;
        drop(reader);

        Ok(())
    }

    #[test]
    fn test_s3_read_sort_write() -> Result<()> {
        let runtime = Arc::new(Builder::new_multi_thread().enable_all().build().unwrap());
        runtime.clone().block_on(async move {
            let common_conf_builder = LakeSoulIOConfigBuilder::new()
                .with_thread_num(2)
                .with_batch_size(8192)
                .with_max_row_group_size(250000)
                .with_object_store_option("fs.s3a.access.key".to_string(), "minioadmin1".to_string())
                .with_object_store_option("fs.s3a.access.secret".to_string(), "minioadmin1".to_string())
                .with_object_store_option("fs.s3a.endpoint".to_string(), "http://localhost:9000".to_string());

            let read_conf = common_conf_builder
                .clone()
                .with_files(vec![
                    "s3://lakesoul-test-bucket/data/native-io-test/large_file.parquet".to_string()
                ])
                .build();
            let mut reader = LakeSoulReader::new(read_conf)?;
            reader.start().await?;

            let schema = reader.schema.clone().unwrap();

            let write_conf = common_conf_builder
                .clone()
                .with_files(vec![
                    "s3://lakesoul-test-bucket/data/native-io-test/large_file_written_sorted.parquet".to_string(),
                ])
                .with_schema_json(serde_json::to_string::<Schema>(schema.borrow()).unwrap())
                .with_primary_keys(vec!["str0".to_string(), "str1".to_string(), "int1".to_string()])
                .build();
            let async_writer = MultiPartAsyncWriter::try_new(write_conf.clone()).await?;
            let schema = async_writer.schema.clone();
            let mut async_writer = SortAsyncWriter::try_new(async_writer, write_conf, schema, runtime.clone())?;

            while let Some(rb) = reader.next_rb().await {
                let rb = rb?;
                async_writer.write_record_batch(rb).await?;
            }

            Box::new(async_writer).flush_and_close().await?;
            drop(reader);

            Ok(())
        })
    }
}
