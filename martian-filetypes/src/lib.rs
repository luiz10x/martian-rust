//!
//! This crate defines filetypes commonly used in bio informatics pipelines.
//!
//! ## Terminology
//! - **Lazy Reading**: Read items one by one from a file that stores a list of items,
//! without reading the entire file into memory
//! - **Lazy Writing**: Write items one by one into a file that stores a list of items.
//!
//! ## Performance comparison
//! There are multiple criterion benchmarks in this crate to help you compare the performance
//! of various read/write operations and their lazy variants. The easiest way to save an in-memory
//! rust type to file is using serde. The following table is meant to be guide to help you choose
//! the format. **The actual performance would depend on the details of the running enviroment and
//! the underlying filesystem. For more insights, run the benchmarks in your environment**
//!
//! The table shows the number of items that you can read/write per second from/into different filetypes
//! using the functionalities provided in this crate. This will obviously be a function of what each
//! item is. For the performance test, each item is a tiny struct with 4 fields.
//!
//! | File Format   | Reading      | Lazy Reading | Writing      | Lazy Writing |
//! | ------------- | ------------ | ------------ | ------------ | ------------ |
//! | `json`        | 815 Kelem/s  | 760 Kelem/s  | 1356 Kelem/s | 1190 Kelem/s |
//! | `json.lz4`    | 327 Kelem/s  | 321 Kelem/s  | 496 Kelem/s  | 806 Kelem/s  |
//! | `bincode`     | 7990 Kelem/s | 7633 Kelem/s | 4896 Kelem/s | 4887 Kelem/s |
//! | `bincode.lz4` | 4554 Kelem/s | 4275 Kelem/s | 4444 Kelem/s | 4363 Kelem/s |
//!
//! ## Examples
//! Look at the individual filetype modules for examples.
//!
//! ## TODO
//! - FastaFile
//! - FastaIndexFile
//! - FastqFile
//! - BamFile
//! - BamIndexFile

use failure::ResultExt;
use martian::{Error, MartianFileType};

use std::fmt;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::string::ToString;

pub mod bin_file;
pub mod json_file;
pub mod lz4_file;

/// Provide context for errors that may arise during read/write
/// of a `MartianFileType`
pub enum ErrorContext<E: ToString> {
    ReadContext(PathBuf, E),
    LazyReadContext(PathBuf, E),
    WriteContext(PathBuf, E),
}

impl<E> fmt::Display for ErrorContext<E>
where
    E: ToString,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            ErrorContext::ReadContext(p, e) => write!(
                formatter,
                "Failed to read MartianFiletype {} due to error: {}",
                p.display(),
                e.to_string()
            ),
            ErrorContext::LazyReadContext(p, e) => write!(
                formatter,
                "Failed to lazy read MartianFiletype {:?} due to error: {:?}",
                p.display(),
                e.to_string()
            ),
            ErrorContext::WriteContext(p, e) => write!(
                formatter,
                "Failed to write to MartianFiletype {:?} due to error: {:?}",
                p.display(),
                e.to_string()
            ),
        }
    }
}

/// A `MartianFileType` `F` is a `FileStorage<T>` if it is valid to
/// save an object of type `T` in a file with the extension `F::extension()`
/// This trait will give us compile time guarantees on whether we are
/// writing into or reading from a file type into an invalid type
pub trait FileStorage<T>: MartianFileType {}

/// A trait that represents a `MartianFileType` that can be read into
/// memory as type `T` or written from type `T`. Use the `read()` and
/// `write()` methods to achieve these.
///
/// If you want to implement this trait for a custom filetype, read
/// the inline comments on which functions are provided and which
/// are required.
pub trait FileTypeIO<T>: MartianFileType + fmt::Debug + FileStorage<T> {
    /// Read the `MartianFileType` as type `T`
    /// The default implementation should work in most cases. It is recommended
    /// **not** to implement this for a custom filetype in general, instead implement
    /// `read_from()`
    fn read(&self) -> Result<T, Error> {
        Ok(<Self as FileTypeIO<T>>::read_from(self.buf_reader()?)
            .with_context(|e| ErrorContext::ReadContext(self.as_ref().into(), e.to_string()))?)
    }

    #[doc(hidden)]
    // In general, do not call this function directly. Use `read()` instead
    // This is the function you need to provide for custom implementations of
    // `FileTypeIO<T>`. Note that the `read()` function is a wrapper around
    // this function. This function essentially describes how you can read the
    // object `T` from a reader. The reason for having this separate from the
    // `read()` function is so that we can extend the functionality by passing
    // in arbitrary readers (for e.g lz4 compressed). See the `lz4_file` for
    // a concrete example
    fn read_from<R: io::Read>(reader: R) -> Result<T, Error>;

    /// Write type `T` into the `MartianFileType`
    /// The default implementation should work in most cases. It is recommended
    /// **not** to implement this for a custom filetype in general, instead implement
    /// `write_into()`.
    fn write(&self, item: &T) -> Result<(), Error> {
        Ok(
            <Self as FileTypeIO<T>>::write_into(self.buf_writer()?, item).with_context(|e| {
                ErrorContext::WriteContext(self.as_ref().into(), e.to_string())
            })?,
        )
    }

    #[doc(hidden)]
    // In general, do not call this function directly. Use `write()` instead.
    // The comments provided in `read_from()` apply here as well.
    fn write_into<W: io::Write>(writer: W, item: &T) -> Result<(), Error>;
}

/// A trait that represents a `MartianFileType` which can be incrementally
/// read or written. For example, you might have a fasta file and you might
/// want to iterate over individual sequences in the file without
/// reading everything into memory at once.
pub trait LazyFileTypeIO<T>: MartianFileType + Sized {
    type Reader: io::Read;
    type Writer: io::Write;

    /// A type that lets you iterate over items of type `T` from a
    /// `MartianFileType` which stores a `Vec<T>`
    type LazyReader: LazyRead<T, Self::Reader, FileType = Self>;

    /// A type that lets you write items of type `T` into a `MartianFileType`
    /// which stores a `Vec<T>`. Implements `LazyWrite` trait
    type LazyWriter: LazyWrite<T, Self::Writer, FileType = Self>;

    /// Get a lazy reader for this `MartianFileType`
    fn lazy_reader(&self) -> Result<Self::LazyReader, Error>;

    /// Consume the reader and read all the items
    fn read_all(&self) -> Result<Vec<T>, Error> {
        let reader = self.lazy_reader()?;
        let mut items = Vec::new();
        for item in reader {
            items.push(item?);
        }
        Ok(items)
    }
    /// Get a lazy writer for this `MartianFileType`
    fn lazy_writer(&self) -> Result<Self::LazyWriter, Error>;
}

/// The trait lazy readers need to implement, which lets you read items one by one from a file
/// that stores a list of items
pub trait LazyRead<T, R: io::Read>: Sized + Iterator<Item = Result<T, Error>> {
    type FileType: MartianFileType;
    fn with_reader(reader: R) -> Result<Self, Error>;
}

/// The trait lazy writers need to implement, which lets you
/// write items one at a time and finish the writing
pub trait LazyWrite<T, W: io::Write>: Sized {
    type FileType: MartianFileType;
    fn with_writer(writer: W) -> Result<Self, Error>;
    /// Lazily write a single item into a writer which stores
    /// a list of items.
    fn write_item(&mut self, item: &T) -> Result<(), Error>;
    /// Finish the lazy writer and return the underlying writer.
    fn finish(self) -> Result<W, Error>;
}

/// Define the lazy writer and lazy reader associated type for a MartianFileType
pub trait LazyAgents<T, W: io::Write, R: io::Read>: Sized + MartianFileType {
    type LazyWriter: LazyWrite<T, W, FileType = Self>;
    type LazyReader: LazyRead<T, R, FileType = Self>;
}

impl<F, T> LazyFileTypeIO<T> for F
where
    F: LazyAgents<T, io::BufWriter<File>, io::BufReader<File>>,
{
    type Writer = io::BufWriter<File>;
    type Reader = io::BufReader<File>;
    type LazyWriter = F::LazyWriter;
    type LazyReader = F::LazyReader;
    fn lazy_reader(&self) -> Result<Self::LazyReader, Error> {
        LazyRead::with_reader(self.buf_reader()?)
    }
    fn lazy_writer(&self) -> Result<Self::LazyWriter, Error> {
        LazyWrite::with_writer(self.buf_writer()?)
    }
}

#[cfg(test)]
pub fn round_trip_check<F, T>(input: &T) -> Result<bool, Error>
where
    F: FileTypeIO<T>,
    T: PartialEq,
{
    // TEST 1: Write as F and read from F
    let pass_direct = {
        let dir = tempfile::tempdir()?;
        let file = F::new(dir.path(), "my_file_roundtrip");
        file.write(input)?;
        let decoded: T = file.read()?;
        input == &decoded
    };

    // TEST 2: Write as Lz4<F> and read from Lz4<F>
    let pass_compressed = {
        let dir = tempfile::tempdir()?;
        let file = lz4_file::Lz4::<F>::new(dir.path(), "my_file_roundtrip_compressed");
        file.write(input)?;
        let decoded: T = file.read()?;
        input == &decoded
    };

    Ok(pass_direct && pass_compressed)
}

#[cfg(test)]
pub fn lazy_round_trip_check<F, T>(input: &Vec<T>, require_read: bool) -> Result<bool, Error>
where
    F: LazyFileTypeIO<T> + FileTypeIO<Vec<T>>,
    lz4_file::Lz4<F>: LazyFileTypeIO<T> + FileTypeIO<Vec<T>>,
    T: PartialEq,
{
    // Write + Lazy read
    let pass_w_lr = {
        let dir = tempfile::tempdir()?;
        let file = F::new(dir.path(), "my_file");
        file.write(input)?;
        let decoded: Vec<T> = file.read_all()?;
        input == &decoded
    };

    // Write + Lazy read [Compressed]
    let pass_w_lr_c = {
        let dir = tempfile::tempdir()?;
        let file = lz4_file::Lz4::<F>::new(dir.path(), "my_file");
        file.write(input)?;
        let decoded: Vec<T> = file.read_all()?;
        input == &decoded
    };

    // Lazy write + read
    let pass_lw_r = if require_read {
        let dir = tempfile::tempdir()?;
        let file = F::new(dir.path(), "my_file");
        let mut lazy_writer = file.lazy_writer()?;
        for item in input {
            lazy_writer.write_item(item)?;
        }
        lazy_writer.finish()?;
        let decoded: Vec<T> = file.read()?;
        input == &decoded
    } else {
        true
    };

    // Lazy write + read [Compressed]
    let pass_lw_r_c = if require_read {
        let dir = tempfile::tempdir()?;
        let file = lz4_file::Lz4::<F>::new(dir.path(), "my_file");
        let mut lazy_writer = file.lazy_writer()?;
        for item in input {
            lazy_writer.write_item(item)?;
        }
        lazy_writer.finish()?;
        let decoded: Vec<T> = file.read()?;
        input == &decoded
    } else {
        true
    };

    // Lazy write + Lazy read
    let pass_lw_lr = {
        let dir = tempfile::tempdir()?;
        let file = F::new(dir.path(), "my_file");
        let mut lazy_writer = file.lazy_writer()?;
        for item in input {
            lazy_writer.write_item(item)?;
        }
        lazy_writer.finish()?;
        let decoded: Vec<T> = file.read_all()?;
        input == &decoded
    };

    // Lazy write + Lazy read [Compressed]
    let pass_lw_lr_c = {
        let dir = tempfile::tempdir()?;
        let file = lz4_file::Lz4::<F>::new(dir.path(), "my_file");
        let mut lazy_writer = file.lazy_writer()?;
        for item in input {
            lazy_writer.write_item(item)?;
        }
        lazy_writer.finish()?;
        let decoded: Vec<T> = file.read_all()?;
        input == &decoded
    };

    Ok(pass_w_lr && pass_w_lr_c && pass_lw_r && pass_lw_r_c && pass_lw_lr && pass_lw_lr_c)
}
