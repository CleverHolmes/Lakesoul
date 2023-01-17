use crate::sorted_merge::utils;

use std::{mem, ptr::null};

use arrow::array::{make_array as make_arrow_array, ArrayData, ArrayDataBuilder, MutableArrayData};
use arrow_buffer::{bit_util, ToByteSlice, Buffer, MutableBuffer};
use arrow_schema::{DataType, Field, IntervalUnit, UnionMode};
use half::f16;

#[derive(Debug)]
pub(crate) struct MergedArrayData {
    pub data_type: DataType,
    pub nullable: bool,
    pub null_count: usize,

    pub len: usize,
    pub null_buffer: MutableBuffer,

    // arrow specification only allows up to 3 buffers (2 ignoring the nulls above).
    // Thus, we place them in the stack to avoid bound checks and greater data locality.
    pub buffer1: MutableBuffer,
    pub buffer2: MutableBuffer,
    // pub child_data: Vec<MutableArrayData<'a>>,
}

impl MergedArrayData {
    pub(crate) fn new(field: &Field, capacity: usize) -> Self {
        Self::with_capacities(field, capacity)
    }

    pub(crate) fn with_capacities(field: &Field, capacity: usize) -> Self {
        let [buffer1, buffer2] = new_buffers(field.data_type(), capacity);
        let nullable = if field.is_nullable() { true } else { false };
        let null_buffer = if nullable {
            let null_bytes = bit_util::ceil(capacity, 8);
            MutableBuffer::from_len_zeroed(null_bytes)
        } else {
            // create 0 capacity mutable buffer with the intention that it won't be used
            MutableBuffer::with_capacity(0)
        };
        Self {
            data_type: (*field.data_type()).clone(),
            nullable: nullable,
            null_count: 0,
            len: 0,
            null_buffer: null_buffer,
            buffer1: buffer1,
            buffer2: buffer2
        }
    }

    pub(crate) fn push_null(&mut self) {
        if !self.nullable { assert!(self.null_buffer.capacity() == 0) };
        // self.extend_null_bit();
        self.len += 1;
        self.null_count += 1;
        // put a default value for None
        let item = utils::get_default_value(&self.data_type);
        println!("[debug][changhui]item's length is {}", item.len());
        self.buffer1.extend_from_slice(item);
    }


    // todo: generilize the fn
    /**
    Upstream needs to use this function to convert data into u8 array and then pass
    unsafe fn any_as_u8_slice<T: Sized>(p: &T) -> &[u8] {
        ::std::slice::from_raw_parts(
            (p as *const T) as *const u8,
            ::std::mem::size_of::<T>(),
        )
    }
    */
    pub(crate) fn push_non_null_item<T: ToByteSlice>(&mut self, item: T) {
        if self.nullable {
            self.extend_non_null_bit();
        }
        match self.data_type {
            DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64 => {
                self.buffer1.push(item); // ensure that the type of t is passed correctly
                self.len += 1;
            },
            _ => panic!("Unsupported DataType: {}", self.data_type)
        }
    }
    // fn push_non_null_item(&mut self, item: &[u8]) {
    //     if self.nullable {
    //         self.extend_non_null_bit();
    //     }
    //     match self.data_type {
    //         DataType::UInt8
    //         | DataType::UInt16
    //         | DataType::UInt32
    //         | DataType::UInt64
    //         | DataType::Int8
    //         | DataType::Int16
    //         | DataType::Int32
    //         | DataType::Int64 => {
    //             self.buffer1.extend_from_slice(item); // ensure that the type of t is passed correctly
    //             self.len += 1;
    //         },
    //         _ => panic!("Unsupported DataType: {}", self.data_type)
    //     }

    // }

    fn extend_non_null_bit(&mut self) {
        utils::resize_for_bits(&mut self.null_buffer, self.len + 1);
        let write_data = self.null_buffer.as_slice_mut();
        bit_util::set_bit(write_data, self.len);
        // self.len += 1;
    }

    pub(crate) fn freeze(self) -> ArrayData {
        let buffers = into_buffers(&self.data_type, self.buffer1, self.buffer2);

        // let child_data = match self.data_type {
        //     DataType::Dictionary(_, _) => vec![dictionary.unwrap()],
        //     _ => {
        //         let mut child_data = Vec::with_capacity(self.child_data.len());
        //         for child in self.child_data {
        //             child_data.push(child.freeze());
        //         }
        //         child_data
        //     }
        // };

        let array_data_builder = ArrayDataBuilder::new(self.data_type)
            .offset(0)
            .len(self.len)
            .null_count(self.null_count)
            .buffers(buffers)
            // .child_data(child_data)
            .null_bit_buffer((self.null_count > 0).then(|| self.null_buffer.into()));

        unsafe { array_data_builder.build_unchecked() }
    }
}

#[inline]
pub(crate) fn new_buffers(data_type: &DataType, capacity: usize) -> [MutableBuffer; 2] {
    let empty_buffer = MutableBuffer::new(0);
    match data_type {
        DataType::Null => [empty_buffer, MutableBuffer::new(0)],
        DataType::Boolean => {
            let bytes = bit_util::ceil(capacity, 8);
            let buffer = MutableBuffer::new(bytes);
            [buffer, empty_buffer]
        }
        DataType::UInt8 => [
            MutableBuffer::new(capacity * mem::size_of::<u8>()),
            empty_buffer,
        ],
        DataType::UInt16 => [
            MutableBuffer::new(capacity * mem::size_of::<u16>()),
            empty_buffer,
        ],
        DataType::UInt32 => [
            MutableBuffer::new(capacity * mem::size_of::<u32>()),
            empty_buffer,
        ],
        DataType::UInt64 => [
            MutableBuffer::new(capacity * mem::size_of::<u64>()),
            empty_buffer,
        ],
        DataType::Int8 => [
            MutableBuffer::new(capacity * mem::size_of::<i8>()),
            empty_buffer,
        ],
        DataType::Int16 => [
            MutableBuffer::new(capacity * mem::size_of::<i16>()),
            empty_buffer,
        ],
        DataType::Int32 => [
            MutableBuffer::new(capacity * mem::size_of::<i32>()),
            empty_buffer,
        ],
        DataType::Int64 => [
            MutableBuffer::new(capacity * mem::size_of::<i64>()),
            empty_buffer,
        ],
        DataType::Float16 => [
            MutableBuffer::new(capacity * mem::size_of::<f32>()),
            empty_buffer,
        ],
        DataType::Float32 => [
            MutableBuffer::new(capacity * mem::size_of::<f32>()),
            empty_buffer,
        ],
        DataType::Float64 => [
            MutableBuffer::new(capacity * mem::size_of::<f64>()),
            empty_buffer,
        ],
        DataType::Date32 | DataType::Time32(_) => [
            MutableBuffer::new(capacity * mem::size_of::<i32>()),
            empty_buffer,
        ],
        DataType::Date64
        | DataType::Time64(_)
        | DataType::Duration(_)
        | DataType::Timestamp(_, _) => [
            MutableBuffer::new(capacity * mem::size_of::<i64>()),
            empty_buffer,
        ],
        DataType::Interval(IntervalUnit::YearMonth) => [
            MutableBuffer::new(capacity * mem::size_of::<i32>()),
            empty_buffer,
        ],
        DataType::Interval(IntervalUnit::DayTime) => [
            MutableBuffer::new(capacity * mem::size_of::<i64>()),
            empty_buffer,
        ],
        DataType::Interval(IntervalUnit::MonthDayNano) => [
            MutableBuffer::new(capacity * mem::size_of::<i128>()),
            empty_buffer,
        ],
        DataType::Utf8 | DataType::Binary => {
            let mut buffer = MutableBuffer::new((1 + capacity) * mem::size_of::<i32>());
            // safety: `unsafe` code assumes that this buffer is initialized with one element
            buffer.push(0i32);
            [buffer, MutableBuffer::new(capacity * mem::size_of::<u8>())]
        }
        DataType::LargeUtf8 | DataType::LargeBinary => {
            let mut buffer = MutableBuffer::new((1 + capacity) * mem::size_of::<i64>());
            // safety: `unsafe` code assumes that this buffer is initialized with one element
            buffer.push(0i64);
            [buffer, MutableBuffer::new(capacity * mem::size_of::<u8>())]
        }
        DataType::List(_) | DataType::Map(_, _) => {
            // offset buffer always starts with a zero
            let mut buffer = MutableBuffer::new((1 + capacity) * mem::size_of::<i32>());
            buffer.push(0i32);
            [buffer, empty_buffer]
        }
        DataType::LargeList(_) => {
            // offset buffer always starts with a zero
            let mut buffer = MutableBuffer::new((1 + capacity) * mem::size_of::<i64>());
            buffer.push(0i64);
            [buffer, empty_buffer]
        }
        DataType::FixedSizeBinary(size) => {
            [MutableBuffer::new(capacity * *size as usize), empty_buffer]
        }
        DataType::Dictionary(child_data_type, _) => match child_data_type.as_ref() {
            DataType::UInt8 => [
                MutableBuffer::new(capacity * mem::size_of::<u8>()),
                empty_buffer,
            ],
            DataType::UInt16 => [
                MutableBuffer::new(capacity * mem::size_of::<u16>()),
                empty_buffer,
            ],
            DataType::UInt32 => [
                MutableBuffer::new(capacity * mem::size_of::<u32>()),
                empty_buffer,
            ],
            DataType::UInt64 => [
                MutableBuffer::new(capacity * mem::size_of::<u64>()),
                empty_buffer,
            ],
            DataType::Int8 => [
                MutableBuffer::new(capacity * mem::size_of::<i8>()),
                empty_buffer,
            ],
            DataType::Int16 => [
                MutableBuffer::new(capacity * mem::size_of::<i16>()),
                empty_buffer,
            ],
            DataType::Int32 => [
                MutableBuffer::new(capacity * mem::size_of::<i32>()),
                empty_buffer,
            ],
            DataType::Int64 => [
                MutableBuffer::new(capacity * mem::size_of::<i64>()),
                empty_buffer,
            ],
            _ => unreachable!(),
        },
        DataType::FixedSizeList(_, _) | DataType::Struct(_) => {
            [empty_buffer, MutableBuffer::new(0)]
        }
        DataType::Decimal128(_, _) | DataType::Decimal256(_, _) => [
            MutableBuffer::new(capacity * mem::size_of::<u8>()),
            empty_buffer,
        ],
        DataType::Union(_, _, mode) => {
            let type_ids = MutableBuffer::new(capacity * mem::size_of::<i8>());
            match mode {
                UnionMode::Sparse => [type_ids, empty_buffer],
                UnionMode::Dense => {
                    let offsets = MutableBuffer::new(capacity * mem::size_of::<i32>());
                    [type_ids, offsets]
                }
            }
        }
    }
}

#[inline]
pub(crate) fn into_buffers(
    data_type: &DataType,
    buffer1: MutableBuffer,
    buffer2: MutableBuffer,
) -> Vec<Buffer> {
    match data_type {
        DataType::Null | DataType::Struct(_) | DataType::FixedSizeList(_, _) => vec![],
        DataType::Utf8
        | DataType::Binary
        | DataType::LargeUtf8
        | DataType::LargeBinary => vec![buffer1.into(), buffer2.into()],
        DataType::Union(_, _, mode) => {
            match mode {
                // Based on Union's DataTypeLayout
                UnionMode::Sparse => vec![buffer1.into()],
                UnionMode::Dense => vec![buffer1.into(), buffer2.into()],
            }
        }
        _ => vec![buffer1.into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lakesoul_reader::ArrowResult;
    use std::sync::Arc;

    use arrow::array::{Int32Array, ArrayData, ArrayRef};
    use arrow::record_batch::RecordBatch;
    use arrow::buffer::Buffer;
    use arrow_schema::{DataType, Schema, SchemaRef};

    // fn demo<T, const N: usize>(v: Vec<T>) -> [T; N] {
    //     v.try_into()
    //         .unwrap_or_else(|v: Vec<T>| panic!("Expected a Vec of length {} but it was {}", N, v.len()))
    // }

    unsafe fn any_as_u8_slice<T: Sized>(p: &T) -> &[u8] {
        ::std::slice::from_raw_parts(
            (p as *const T) as *const u8,
            ::std::mem::size_of::<T>(),
        )
    }

    fn fill_value_for_primitive(array_data: &mut MergedArrayData, dt: &DataType, item: i32) {
        match *dt {
            DataType::UInt8 => array_data.push_non_null_item(item as u8),
            DataType::UInt16 => array_data.push_non_null_item(item as u16),
            DataType::UInt32 => array_data.push_non_null_item(item as u32),
            DataType::UInt64 => array_data.push_non_null_item(item as u64),
            DataType::Int8 => array_data.push_non_null_item(item as i8),
            DataType::Int16 => array_data.push_non_null_item(item as i16),
            DataType::Int32 => array_data.push_non_null_item(item as i32),
            DataType::Int64 => array_data.push_non_null_item(item as i64),
            _ => panic!("Unsupported DataType: {}", dt)
        }
    } 

    fn _test_primitive_push(field_name: &str, dt: DataType, nullable: bool) {
        let field = Field::new(field_name, dt.clone(), nullable);
        let mut array_data = MergedArrayData::new(&field, 5);
        println!("[debug][changhui]MergedArrayData init: {:?}", array_data);
        if nullable {
            for i in 0..5 {
                if i % 2 == 0 {
                    fill_value_for_primitive(&mut array_data, &dt, i);
                } else {
                    array_data.push_null();
                }
            }
        } else {
            for i in 0..5 {
                fill_value_for_primitive(&mut array_data, &dt, i);
            }
        }
        let ad = array_data.freeze();
        println!("{:?}", ad);
        let column = make_arrow_array(ad);
        let schema = Schema::new(vec![field]);
        let rb = RecordBatch::try_new(std::sync::Arc::new(schema), vec![column,]).unwrap();
        assert_eq!(rb.column(0).null_count(), if nullable {2} else {0});
        assert_eq!(rb.num_rows(), 5);
        println!("{:?}", rb);
    }

    #[test]
    fn test_primitive_data_type() {
        _test_primitive_push("int32", DataType::Int32, true);
        _test_primitive_push("int64", DataType::Int64, false);
        _test_primitive_push("uint16", DataType::UInt16, false);
    }

    #[test]
    fn test_builder() {
        // Buffer needs to be at least 25 long
        let v = (0..25).collect::<Vec<i32>>();
        let b1 = Buffer::from_slice_ref(&v);
        let arr_data = ArrayData::builder(DataType::Int32)
            .len(20)
            .offset(5)
            .add_buffer(b1)
            .null_bit_buffer(Some(Buffer::from(vec![
                0b01011111, 0b10110101, 0b01100011, 0b00011110,
            ])))
            .build()
            .unwrap();

        assert_eq!(20, arr_data.len());
        assert_eq!(10, arr_data.null_count());
        assert_eq!(5, arr_data.offset());
        assert_eq!(1, arr_data.buffers().len());
        assert_eq!(
            Buffer::from_slice_ref(&v).as_slice(),
            arr_data.buffers()[0].as_slice()
        );
    }
}