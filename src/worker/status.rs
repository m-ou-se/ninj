use raw_string::RawStr;

/// Something that a [`Worker`] can report its status to.
pub trait StatusListener {
	/// This will be called for every [update][StatusUpdater::update] from a
	/// worker.
	fn status_update(&self, worker_id: usize, event: StatusEvent);
}

/// A status update from a worker to a [`StatusListener`].
pub enum StatusEvent<'a> {
	Idle,
	Running { task: usize },
	Done,
	Failed,
	Output { task: usize, data: &'a RawStr },
}

/// Reports status updates of a worker to a [`StatusListener`].
#[derive(Clone, Copy)]
pub struct StatusUpdater<'a> {
	pub status_listener: &'a (dyn StatusListener + Sync),
	pub worker_id: usize,
}

impl<'a> StatusUpdater<'a> {
	/// Report a new status for this worker.
	pub fn update(&self, event: StatusEvent) {
		self.status_listener.status_update(self.worker_id, event);
	}
}
