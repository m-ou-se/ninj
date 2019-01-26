use nix::poll::{poll, EventFlags, PollFd};
use std::fs::File;
use std::io::{Read, Result as IoResult};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::process::{Child, ExitStatus};
use std::slice::from_raw_parts_mut;

/// Waits for a [`Child`] to finish, while reading its output live as it runs.
///
/// Captures stdout and/or stderr, if they are set to [`piped`][Stdio::piped].
///
/// After `timeout_ms` milliseconds of silence (or when the output switches
/// between stdout and stderr or back), `output_callback` is called with the
/// captured output.
///
///
/// Waits for the child to exit, and returns its [`ExitStatus`].
///
/// # Example
///
/// ```ignore
/// use raw_string::RawStr;
///
/// let child = Command::new("cargo")
///   .stdin(Stdio::null())
///   .stdout(Stdio::piped())
///   .stderr(Stdio::piped())
///   .arg("build")
///   .spawn()?;
///
/// let result = listen_to_child(child, 100, |is_stderr, buffer| {
///    println!("{}", RawStr::from(buffer));
/// })?;
///
/// println!("Subprocess exited: {}", result);
/// ```
pub fn listen_to_child(
	mut child: Child,
	timeout_ms: i32,
	output_callback: &dyn Fn(Source, &[u8]),
) -> IoResult<ExitStatus> {
	// The file descriptors we'll be reading from.
	let mut fds = [
		child.stdout.take().map(|f| unsafe { into_file(f) }),
		child.stderr.take().map(|f| unsafe { into_file(f) }),
	];

	// The list of file descriptors `poll` will need to check. (In the same
	// order as `fds`.)
	let mut poll_fds = [
		PollFd::new(fds[0].as_ref().unwrap().as_raw_fd(), EventFlags::POLLIN),
		PollFd::new(fds[1].as_ref().unwrap().as_raw_fd(), EventFlags::POLLIN),
	];

	// Data that has been read from one of the pipes.
	let mut buffer = Vec::<u8>::with_capacity(16 * 1024);

	// The stream from which the data in `buffer` came.
	// (To avoid mixing the different streams.)
	let mut buffer_source = Source::Stdout;

	loop {
		// Only look at stdout if that stream is still open.
		let start = if fds[0].is_some() { 0 } else { 1 };

		// Only look at stderr if that stream is still open.
		let end = if fds[1].is_some() { 2 } else { 1 };

		// If both are closed, we reading them.
		if start == end {
			break;
		}

		let timeout_ms = if buffer.is_empty() {
			-1
		} else {
			// If there's data in the buffer, we should output it after
			// `timeout_ms` milliseconds of silence.
			timeout_ms
		};

		// Wait until there's data to read, or the timeout occurs.
		if poll(&mut poll_fds[start..end], timeout_ms).map_err(|e| e.as_errno().unwrap())? == 0 {
			// Timeout.
			// Flush the buffer.
			output_callback(buffer_source, &buffer);
			buffer.clear();
		} else {
			// New data (or errors) available.
			for i in start..end {
				let source = match i {
					0 => Source::Stdout,
					_ => Source::Stderr,
				};

				if poll_fds[i].revents().unwrap().contains(EventFlags::POLLIN) {
					if source != buffer_source {
						// Switch from stdout to stderr or back.
						if !buffer.is_empty() {
							// Flush the buffer first.
							output_callback(buffer_source, &buffer);
							buffer.clear();
						}
						buffer_source = source;
					}

					// Reserve 4 KiB of space in the buffer.
					buffer.reserve(4 * 1024);

					// The unused (free) part of the buffer.
					let buffer_free_space = unsafe {
						from_raw_parts_mut(
							buffer.as_mut_ptr().add(buffer.len()),
							buffer.capacity() - buffer.len(),
						)
					};

					// Read bytes, and ignore any errors.
					// Errors are handled by checking `revents` for POLLERR.
					let n_read = fds[i]
						.as_mut()
						.unwrap()
						.read(buffer_free_space)
						.unwrap_or(0);

					// Make the read bytes part of the buffer.
					let new_len = buffer.len() + n_read;
					unsafe { buffer.set_len(new_len) };
				}

				if poll_fds[i]
					.revents()
					.unwrap()
					.intersects(EventFlags::POLLERR | EventFlags::POLLHUP)
				{
					// Close our side of the pipe which was closed by the client.
					fds[i].take();
				}
			}
		}
	}

	// Flush the buffer, if there's anything in there.
	if !buffer.is_empty() {
		output_callback(buffer_source, &buffer);
		buffer.clear();
	}

	// Both stderr and stdout have been closed. Now we just wait for the process to
	// exit.
	child.wait()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
	Stdout,
	Stderr,
}

unsafe fn into_file(stream: impl IntoRawFd) -> File {
	File::from_raw_fd(stream.into_raw_fd())
}
