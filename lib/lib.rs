//! This library crate contains all the re-usable parts of `ninj`, an
//! implementation of the `ninja` build system.
//!
//! # File formats
//!
//! This crate implements support for several file formats:
//!
//! - **`build.ninja` files**
//!
//!   The [`spec`] module contains everything you need to parse `build.ninja`
//!   files, including variable expansion, traversing other ninja files, and
//!   resolving build rules.
//!
//! - **`.ninja_log` files**
//!
//!   The [`buildlog`] module allows both reading from and writing to
//!   `.ninja_log` files,   which store how each target was built previously.
//!
//! - **`.ninja_deps` files**
//!
//!   The [`deplog`] module can read and write `.ninja_deps` files, which hold
//!   the dependency information discovered during previous builds.
//!
//! - **`Makefile`-style dependency files**
//!
//!   The [`depfile`] module can read `Makefile`-style dependency files which
//!   are written by some compilers, such as GCC and Clang.
//!
//! # Utilities
//!
//! Other than file formats, this crate also provides the following utilities:
//!
//! - **Error handling**
//!
//!   [`ErrorWithLocation`][error::ErrorWithLocation] adds location information
//!   (file name and line number) to an [`Error`][std::error::Error], so that
//!   the user can know where something went wrong while reading/parsing a file.
//!
//! - **A 'build queue'**
//!
//!   [`BuildQueue`](queue::BuildQueue) can track tasks and their dependencies,
//!   and will tell you which tasks need to be run in what order.
//!
//! - **Reading of `mtime`s**
//!
//!   The [`mtime`] module contains an [`mtime`][mtime::mtime] function, but
//!   also has a [`StatCache`][mtime::StatCache] which helps to reducing the
//!   number of `stat()` syscalls.
//!
//! - **Checking for outdated targets**
//!
//!   The [`outdated`] module contains an [`is_outdated`][outdated::is_outdated]
//!   function, which checks the `mtime`'s of targets and their dependencies
//!   to check if it should be rebuilt.

pub mod buildlog;
pub mod depfile;
pub mod deplog;
pub mod error;
pub mod mtime;
pub mod outdated;
pub mod queue;
pub mod spec;
