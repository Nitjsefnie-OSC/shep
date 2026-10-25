//! The descriptors a handover carried, by the key they were named under.

use core::fmt;

/// The descriptors a predecessor carried, waiting to be claimed by key.
///
/// Whatever is not claimed is closed when this is dropped.
///
/// Empty on a platform with no `execve`, where nothing can be carried.
#[derive(Default)]
pub struct Carried {
    #[cfg(unix)]
    fds: std::collections::BTreeMap<String, std::os::fd::OwnedFd>,
}

impl fmt::Debug for Carried {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut keys = f.debug_list();
        #[cfg(unix)]
        keys.entries(self.fds.keys());
        keys.finish()
    }
}

#[cfg(unix)]
impl Carried {
    pub(crate) const fn new(fds: std::collections::BTreeMap<String, std::os::fd::OwnedFd>) -> Self {
        Self { fds }
    }

    /// Claim the descriptor carried as `key`, or `None` if nothing was carried
    /// under it or it was already claimed.
    pub fn take(&mut self, key: &str) -> Option<CarriedFd> {
        self.fds.remove(key).map(CarriedFd)
    }

    /// The keys still waiting to be claimed, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.fds.keys().map(String::as_str)
    }

    /// How many descriptors are still waiting to be claimed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fds.len()
    }

    /// Whether nothing is waiting to be claimed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fds.is_empty()
    }
}

/// One carried descriptor, not yet turned into the thing it was.
///
/// The `into_*` conversions that return async types register with the Tokio
/// runtime that is current, so call them from inside one.
#[cfg(unix)]
pub struct CarriedFd(std::os::fd::OwnedFd);

#[cfg(unix)]
impl fmt::Debug for CarriedFd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use std::os::fd::AsRawFd as _;
        f.debug_tuple("CarriedFd")
            .field(&self.0.as_raw_fd())
            .finish()
    }
}

#[cfg(unix)]
impl CarriedFd {
    /// The descriptor itself, for a kind of object this crate has no async
    /// type for.
    #[must_use]
    pub fn into_owned_fd(self) -> std::os::fd::OwnedFd {
        self.0
    }

    /// The read end of a pipe, as an async reader. The descriptor is put in
    /// non-blocking mode.
    ///
    /// # Errors
    ///
    /// The descriptor is not the read end of a pipe, or there is no Tokio
    /// runtime.
    pub fn into_reader(self) -> std::io::Result<tokio::net::unix::pipe::Receiver> {
        tokio::net::unix::pipe::Receiver::from_owned_fd(self.0)
    }

    /// The write end of a pipe, as an async writer. The descriptor is put in
    /// non-blocking mode.
    ///
    /// # Errors
    ///
    /// The descriptor is not the write end of a pipe, or there is no Tokio
    /// runtime.
    pub fn into_writer(self) -> std::io::Result<tokio::net::unix::pipe::Sender> {
        tokio::net::unix::pipe::Sender::from_owned_fd(self.0)
    }

    /// A connected unix socket, as an async stream that reads and writes. The
    /// descriptor is put in non-blocking mode.
    ///
    /// # Errors
    ///
    /// The descriptor could not be made non-blocking, or there is no Tokio
    /// runtime.
    pub fn into_stream(self) -> std::io::Result<tokio::net::UnixStream> {
        let stream = std::os::unix::net::UnixStream::from(self.0);
        stream.set_nonblocking(true)?;
        tokio::net::UnixStream::from_std(stream)
    }
}
