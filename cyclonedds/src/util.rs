use std::io::Read;

/// A reader for a list of scatter gather buffers
pub(crate) struct SGReader<'a> {
    sc_list: Option<&'a [&'a [u8]]>,
    //the current slice that is used
    slice_cursor: usize,
    //the current offset within the slice
    slice_offset: usize,
}

impl<'a> SGReader<'a> {
    pub fn new(sc_list: &'a [&'a [u8]]) -> Self {
        SGReader {
            sc_list: Some(sc_list),
            slice_cursor: 0,
            slice_offset: 0,
        }
    }

    pub fn into_vec(mut self, size: usize) -> std::io::Result<Vec<u8>> {
        let mut v = vec![0u8; size];
        self.read_exact(v.as_mut_slice())?;
        Ok(v)
    }
}

impl<'a> Read for SGReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read_buf_len = buf.len();
        if let Some(sc_list) = self.sc_list.as_ref() {
            let source_slice = sc_list[self.slice_cursor];
            let num_slices = sc_list.len();
            let source_slice_rem = source_slice.len() - self.slice_offset;
            let source_slice = &source_slice[self.slice_offset..];

            let copy_length = std::cmp::min(source_slice_rem, read_buf_len);

            //copy the bytes, lengths have to be the same
            buf[..copy_length].copy_from_slice(&source_slice[..copy_length]);

            if copy_length == source_slice_rem {
                // we have completed this slice. move to the next
                self.slice_cursor += 1;
                self.slice_offset = 0;

                if self.slice_cursor >= num_slices {
                    //no more slices, invalidate the sc_list
                    let _ = self.sc_list.take();
                }
            } else {
                // we have not completed the current slice, just bump up the slice offset
                self.slice_offset += copy_length;
            }

            Ok(copy_length)
        } else {
            // No more data
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    // 分散されたデータを連結して読み取るテスト
    #[test]
    fn scatter_gather() {
        let a = vec![1, 2, 3, 4, 5, 6];
        let b = vec![7, 8, 9, 10, 11];
        let c = vec![12, 13, 14, 15];
        let d = vec![16, 17, 18, 19, 20, 21];

        let sla = unsafe { std::slice::from_raw_parts(a.as_ptr(), a.len()) };
        let slb = unsafe { std::slice::from_raw_parts(b.as_ptr(), b.len()) };
        let slc = unsafe { std::slice::from_raw_parts(c.as_ptr(), c.len()) };
        let sld = unsafe { std::slice::from_raw_parts(d.as_ptr(), d.len()) };

        let sc_list = vec![sla, slb, slc, sld];

        let mut reader = SGReader::new(&sc_list);

        // readメソッドで分散データを連結して読み取る
        let mut buf = vec![0, 0, 0, 0, 0];
        if let Ok(n) = reader.read(&mut buf) {
            assert_eq!(&buf[..n], vec![1, 2, 3, 4, 5]);
        } else {
            panic!("should not panic");
        }
        if let Ok(n) = reader.read(&mut buf) {
            assert_eq!(&buf[..n], vec![6]);
        } else {
            panic!("should not panic");
        }
    }
}
