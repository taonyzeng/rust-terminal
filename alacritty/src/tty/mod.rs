
//! tty related functionality

use mio;
use std::io;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use self::unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

/// This trait defines the behaviour needed to read and/or write to a stream.
/// It defines an abstraction over mio's interface in order to allow either one
/// read/write object or a seperate read and write object.
pub trait EventedReadWrite {
    type Reader: io::Read;
    type Writer: io::Write;

    fn register(&mut self, &mio::Poll, &mut Iterator<Item = &usize>, mio::Ready, mio::PollOpt) -> io::Result<()>;
    fn reregister(&mut self, &mio::Poll, mio::Ready, mio::PollOpt) -> io::Result<()>;
    fn deregister(&mut self, &mio::Poll) -> io::Result<()>;

    fn reader(&mut self) -> &mut Self::Reader;
    fn read_token(&self) -> mio::Token;
    fn writer(&mut self) -> &mut Self::Writer;
    fn write_token(&self) -> mio::Token;
}
