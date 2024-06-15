use std::fs::File;
use std::io::{BufRead, BufReader, Error, ErrorKind, Read, Result};

use bytes::{Buf as _, Bytes};
use tokio::runtime::Handle;

#[repr(transparent)]
pub struct DocumentStream(BufReader<CompressionAdapter<SourceAdapter>>);

impl DocumentStream {
    pub fn new(inner: CompressionAdapter<SourceAdapter>) -> Self {
        DocumentStream(BufReader::new(inner))
    }
}

impl Read for DocumentStream {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf)
    }
}
impl BufRead for DocumentStream {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        self.0.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.0.consume(amt)
    }
}

pub enum CompressionAdapter<R: Read> {
    Normal(R),
    Decompressed(bzip2::read::BzDecoder<R>),
}

impl<R: Read> CompressionAdapter<R> {
    pub fn new_passthrough(inner: R) -> Self {
        CompressionAdapter::Normal(inner)
    }

    pub fn new_bzip2(inner: R) -> Self {
        CompressionAdapter::Decompressed(bzip2::read::BzDecoder::<R>::new(inner))
    }
}

impl<R: Read> Read for CompressionAdapter<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            CompressionAdapter::Normal(pass) => pass.read(buf),
            CompressionAdapter::Decompressed(pass) => pass.read(buf),
        }
    }
}

pub enum SourceAdapter {
    Local(BufReader<File>),
    Remote {
        resp: reqwest::Response,
        buffer: Bytes,
        pos: usize,
        runtime: Handle,
    },
}

impl Read for SourceAdapter {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self {
            SourceAdapter::Local(pass) => pass.read(buf),
            SourceAdapter::Remote {
                resp,
                buffer,
                pos,
                runtime,
            } => {
                if buffer.is_empty() || *pos >= buffer.len() {
                    let next_chunk = resp.chunk();
                    let next_chunk = match runtime.block_on(next_chunk) {
                        Ok(it) => it,
                        Err(err) => return Err(Error::new(ErrorKind::ConnectionAborted, err)),
                    };
                    *buffer = match next_chunk {
                        Some(it) => it,
                        None => {
                            return {
                                log::trace!("End of stream");
                                Ok(0)
                            }
                        }
                    };
                    *pos = 0;
                }
                let copy_len = (buffer.len() - *pos).min(buf.len());
                buffer.slice(*pos..).copy_to_slice(&mut buf[..copy_len]);
                *pos += copy_len;
                Ok(copy_len)
            }
        }
    }
}

impl BufRead for SourceAdapter {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        match self {
            SourceAdapter::Local(pass) => pass.fill_buf(),
            SourceAdapter::Remote {
                resp,
                buffer,
                pos,
                runtime,
            } => {
                if buffer.is_empty() || *pos >= buffer.len() {
                    let next_chunk = resp.chunk();
                    let next_chunk = match runtime.block_on(next_chunk) {
                        Ok(it) => it,
                        Err(err) => return Err(Error::new(ErrorKind::ConnectionAborted, err)),
                    };
                    *buffer = match next_chunk {
                        Some(it) => it,
                        None => return Ok(&[0]),
                    };
                    *pos = 0;
                }

                let result = unsafe {
                    let addr = std::ptr::addr_of!(buffer[0]);
                    let addr = addr.add(*pos);
                    std::slice::from_raw_parts(addr, buffer.len() - *pos - 1)
                };

                Ok(result)
            }
        }
    }

    fn consume(&mut self, amt: usize) {
        match self {
            SourceAdapter::Local(pass) => pass.consume(amt),
            SourceAdapter::Remote { pos, .. } => {
                *pos += amt;
            }
        }
    }
}
