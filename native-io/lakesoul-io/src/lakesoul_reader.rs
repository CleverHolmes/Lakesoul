use atomic_refcell::AtomicRefCell;
use std::collections::HashMap;
use std::mem::MaybeUninit;
use std::mem::ManuallyDrop;
use std::num::NonZeroUsize;
use std::sync::Arc;

use std::time::Instant; 

use derivative::Derivative;


pub use datafusion::arrow::error::ArrowError;
pub use datafusion::arrow::error::Result as ArrowResult;
pub use datafusion::arrow::record_batch::RecordBatch;
pub use datafusion::error::{DataFusionError, Result};
use datafusion::execution::runtime_env::{RuntimeConfig, RuntimeEnv};
use datafusion::logical_expr::Expr;
use datafusion::physical_plan::SendableRecordBatchStream;
use datafusion::prelude::{SessionConfig, SessionContext};
use object_store::aws;

use tokio::runtime::{Builder, Runtime};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::merge_logic::merge_partitioned_file::MergePartitionedFile;

#[derive(Derivative, Default)]
pub struct LakeSoulReaderConfig {
    // files to read
    files: Vec<String>,
    merge_files: Vec<MergePartitionedFile>,
    // primary key column names
    primary_keys: Vec<String>,
    // selecting columns
    columns: Vec<String>,
    // filtering predicates
    filters: Vec<Expr>,
    batch_size: usize,

    // object store related configs
    object_store_options: HashMap<String, String>,

    // tokio runtime related configs
    #[derivative(Default(value = "2"))]
    thread_num: usize,
}

pub struct LakeSoulReaderConfigBuilder {
    config: LakeSoulReaderConfig,
}

impl LakeSoulReaderConfigBuilder {
    pub fn new() -> Self {
        LakeSoulReaderConfigBuilder {
            config: LakeSoulReaderConfig::default(),
        }
    }

    pub fn with_file(mut self, file: String) -> Self {
        self.config.files.push(file);
        self
    }

    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.config.files = files;
        self
    }

    pub fn with_merge_file(mut self, file: MergePartitionedFile) -> Self {
        self.config.merge_files.push(file);
        self
    }

    pub fn with_merge_files(mut self, files: Vec<MergePartitionedFile>) -> Self {
        self.config.merge_files = files;
        self
    }

    pub fn with_primary_keys(mut self, pks: Vec<String>) -> Self {
        self.config.primary_keys = pks;
        self
    }

    pub fn with_column(mut self, col: String) -> Self {
        self.config.columns.push(col);
        self
    }

    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.config.batch_size=batch_size;
        self
    }


    pub fn with_columns(mut self, cols: Vec<String>) -> Self {
        self.config.columns = cols;
        self
    }

    pub fn with_filters(mut self, filters: Vec<Expr>) -> Self {
        self.config.filters = filters;
        self
    }

    pub fn with_object_store_option(mut self, key: String, value: String) -> Self {
        self.config.object_store_options.insert(key, value);
        self
    }

    pub fn with_thread_num(mut self, thread_num: usize) -> Self {
        self.config.thread_num = thread_num;
        self
    }

    pub fn build(self) -> LakeSoulReaderConfig {
        self.config
    }
}

pub struct LakeSoulReader {
    sess_ctx: SessionContext,
    config: LakeSoulReaderConfig,
    stream: Box<MaybeUninit<SendableRecordBatchStream>>,
}

impl LakeSoulReader {
    pub fn new(config: LakeSoulReaderConfig) -> Result<Self> {
        let sess_ctx = LakeSoulReader::create_session_context(&config)?;
        Ok(LakeSoulReader {
            sess_ctx,
            config,
            stream: Box::new_uninit(),
        })
    }

    fn check_fs_type_enabled(config: &LakeSoulReaderConfig, fs_name: &str) -> bool {
        if let Some(fs_enabled) = config
            .object_store_options
            .get(format!("fs.{}.enabled", fs_name).as_str())
        {
            return match fs_enabled.parse::<bool>() {
                Ok(enabled) => enabled,
                _ => false,
            };
        }
        false
    }

    fn register_s3_object_store(config: &LakeSoulReaderConfig, runtime: &RuntimeEnv) -> Result<()> {
        if !LakeSoulReader::check_fs_type_enabled(config, "s3") {
            return Ok(());
        }
        let key = config.object_store_options.get("fs.s3.access.key");
        let secret = config.object_store_options.get("fs.s3.access.secret");
        let region = config.object_store_options.get("fs.s3.region");
        let bucket = config.object_store_options.get("fs.s3.bucket");

        if region == None {
            return Err(DataFusionError::ArrowError(ArrowError::InvalidArgumentError(
                "missing fs.s3.region".to_string(),
            )));
        }

        if bucket == None {
            return Err(DataFusionError::ArrowError(ArrowError::InvalidArgumentError(
                "missing fs.s3.bucket".to_string(),
            )));
        }

        let endpoint = config.object_store_options.get("fs.s3.endpoint");
        let s3_store = aws::new_s3(
            key,
            secret,
            region.unwrap(),
            bucket.unwrap(),
            endpoint,
            None::<String>,
            NonZeroUsize::new(4).unwrap(),
            true,
        )?;
        runtime.register_object_store("s3", bucket.unwrap(), Arc::new(s3_store));
        Ok(())
    }

    fn create_session_context(config: &LakeSoulReaderConfig) -> Result<SessionContext> {
        let sess_conf = SessionConfig::default()
            .with_batch_size(config.batch_size);
        let runtime = RuntimeEnv::new(RuntimeConfig::new())?;

        // register object store(s)
        LakeSoulReader::register_s3_object_store(config, &runtime)?;

        // create session context
        Ok(SessionContext::with_config_rt(sess_conf, Arc::new(runtime)))
    }

    pub async fn start(&mut self) -> Result<()> {
        let mut df = self
            .sess_ctx
            .read_parquet(self.config.files[0].as_str(), Default::default())
            .await?;
        if !self.config.columns.is_empty() {
            let cols: Vec<_> = self.config.columns.iter().map(String::as_str).collect();
            df = df.select_columns(&cols)?;
        }
        df = self.config.filters.iter().try_fold(df, |df, f| df.filter(f.clone()))?;
        self.stream = Box::new(MaybeUninit::new(df.execute_stream().await?));
        Ok(())
    }

    pub async fn next_rb(&mut self) -> Option<ArrowResult<RecordBatch>> {
        unsafe { self.stream.assume_init_mut().next().await }
    }


}

// Reader will be used in async closure sent to tokio
// while accessing its mutable methods.
pub struct SyncSendableMutableLakeSoulReader {
    inner: Arc<AtomicRefCell<Mutex<LakeSoulReader>>>,
    runtime: Arc<Runtime>,
}

impl SyncSendableMutableLakeSoulReader {
    pub fn new(reader: LakeSoulReader, runtime:Runtime) -> Self {
        SyncSendableMutableLakeSoulReader {
            inner: Arc::new(AtomicRefCell::new(Mutex::new(reader))),
            runtime: Arc::new(runtime)
        }
    }

    pub fn start_blocked(&self) -> Result<()> {
        let inner_reader = self.inner.clone();
        let runtime = self.get_runtime();
        runtime.block_on(async { inner_reader.borrow().lock().await.start().await })
    }

    pub fn next_rb_callback(
        &self,
        f: Box<dyn FnOnce(Option<ArrowResult<RecordBatch>>) + Send + Sync>,
    ) -> JoinHandle<()> {
        let inner_reader = self.get_inner_reader();
        let runtime = self.get_runtime();
        runtime.spawn(async move {
            let reader = inner_reader.borrow();
            let mut reader = reader.lock().await;
            f(reader.next_rb().await);
        })
    }

    pub fn hello(&self) {
    }

    fn get_runtime(&self) -> Arc<Runtime> {
        self.runtime.clone()
    }

    fn get_inner_reader(&self) -> Arc<AtomicRefCell<Mutex<LakeSoulReader>>>{
        self.inner.clone()
    }
}


impl Iterator for LakeSoulReader{
    type Item=RecordBatch;
    fn next(&mut self) -> Option<RecordBatch>{
        None
    }
}

impl Drop for LakeSoulReader{
    fn drop(&mut self) {
        println!("Dropping LakeSoulReader with data `{}`!", self.config.thread_num);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::util::pretty::print_batches;
    use std::sync::mpsc::sync_channel;

    #[tokio::test]
    async fn test_reader_local() -> Result<()> {
        let reader_conf = LakeSoulReaderConfigBuilder::new()
            .with_files(vec![
                // "/Users/ceng/base-0-0.parquet"
                // "/Users/ceng/part-00003-68b546de-5cc6-4abb-a8a9-f6af2e372791-c000.snappy.parquet"
                "/Users/ceng/Documents/GitHub/LakeSoul/native-io/lakesoul-io-java/src/test/resources/sample-parquet-files/part-00000-a9e77425-5fb4-456f-ba52-f821123bd193-c000.snappy.parquet"
                .to_string()])
            .with_thread_num(1)
            .with_batch_size(256)
            .build();
        let mut reader = LakeSoulReader::new(reader_conf)?;
        let mut reader = ManuallyDrop::new(reader);
        reader.start().await?;
        static mut row_cnt: usize = 0;

        while let Some(rb) = reader.next_rb().await {
            // print_batches(std::slice::from_ref(&record_batch?))?;
            let num_rows = &rb.unwrap().num_rows();
            unsafe {
                row_cnt = row_cnt + num_rows;
                println!("{}", row_cnt);
            }
        }
        // unsafe{
        //     ManuallyDrop::drop(&mut reader);
        // }
        Ok(())
    }

    #[test]
    fn test_reader_local_blocked() -> Result<()> {
        let reader_conf = LakeSoulReaderConfigBuilder::new()
            .with_files(vec![
                "/Users/ceng/part-00003-68b546de-5cc6-4abb-a8a9-f6af2e372791-c000.snappy.parquet"
                // "/Users/ceng/Documents/GitHub/LakeSoul/native-io/lakesoul-io-java/src/test/resources/sample-parquet-files/part-00000-a9e77425-5fb4-456f-ba52-f821123bd193-c000.snappy.parquet"
                    .to_string(),
            ])
            .with_thread_num(1)
            .with_batch_size(8192)
            .build();
        let reader = LakeSoulReader::new(reader_conf)?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(reader.config.thread_num)
            .build()
            .unwrap();
        let reader = SyncSendableMutableLakeSoulReader::new(reader, runtime);
        reader.start_blocked()?;
        static mut row_cnt: usize = 0;
        loop {
            let (tx, rx) = sync_channel(1);
            let start = Instant::now();
            let f = move |rb: Option<ArrowResult<RecordBatch>>| match rb {
                None => tx.send(true).unwrap(),
                Some(rb) => {
                    // print_batches(std::slice::from_ref(&rb.unwrap())).unwrap();
                    let num_rows = &rb.unwrap().num_rows();
                    unsafe {
                        row_cnt = row_cnt + num_rows;
                        println!("{}", row_cnt);
                    }
                    println!("time cost: {:?} ms", start.elapsed().as_millis());// ms
                    tx.send(false).unwrap();
                }
            };
            reader.next_rb_callback(Box::new(f));
            let done = rx.recv().unwrap();
            if done {
                break;
            }
        }
        Ok(())
    }
}
