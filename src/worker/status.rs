use raw_string::RawStr;
use std::mem::forget;

/// Something that a [`Worker`] can report its status to.
pub trait StatusListener {
	/// This will be called for every [update][WorkerStatusUpdater::update] from
	/// a worker.
	fn update(&self, worker_id: usize, update: WorkerUpdate);
}

/// A status update from a worker to a [`StatusListener`].
#[derive(Clone, Copy, Debug)]
pub enum WorkerUpdate<'a> {
	/// Something happened with a task the worker is working on.
	Task {
		/// The task which this update is about.
		task_id: usize,
		/// What happened with the task.
		update: TaskUpdate<'a>,
	},
	/// The worker is waiting for tasks to become ready.
	Idle,
	/// The worker is done, as there are no more tasks to do.
	Done,
}

/// A status update about a specific task.
#[derive(Clone, Copy, Debug)]
pub enum TaskUpdate<'a> {
	/// The task started to execute.
	Started,
	/// The task's running command produced output.
	Output { data: &'a RawStr },
	/// The task finished succesfully.
	Succeeded,
	/// The task failed.
	Failed,
}

/// Reports status updates of a worker to a [`StatusListener`].
pub struct WorkerStatusUpdater<'a> {
	pub status_listener: &'a (dyn StatusListener + Sync),
	pub worker_id: usize,
}

/// Reports status updates of a task of a worker to a [`StatusListener`].
pub struct TaskStatusUpdater<'a> {
	worker_status_updater: &'a WorkerStatusUpdater<'a>,
	task_id: usize,
}

impl<'a> WorkerStatusUpdater<'a> {
	/// Report starting a new task, and get the corresponding
	/// [`TaskStatusUpdater`].
	///
	/// Dropping the returned object without calling
	/// [`succeeded`][TaskStatusUpdater::succeeded] will mark the task as
	/// failed.
	pub fn start_task(&self, task_id: usize) -> TaskStatusUpdater {
		let updater = TaskStatusUpdater {
			worker_status_updater: self,
			task_id,
		};
		updater.send_update(TaskUpdate::Started);
		updater
	}

	/// Mark the worker as idle.
	pub fn idle(&self) {
		self.send_update(WorkerUpdate::Idle);
	}

	/// Mark the task as done.
	///
	/// Dropping the [`WorkerStatusUpdater`] has the same effect.
	pub fn done(self) {}

	fn send_update(&self, update: WorkerUpdate) {
		self.status_listener.update(self.worker_id, update);
	}
}

impl<'a> Drop for WorkerStatusUpdater<'a> {
	fn drop(&mut self) {
		self.send_update(WorkerUpdate::Done);
	}
}

impl<'a> TaskStatusUpdater<'a> {
	/// Report new output from the running task.
	pub fn output(&self, data: &RawStr) {
		self.send_update(TaskUpdate::Output { data });
	}

	/// Mark the task as succeeded.
	pub fn succeeded(self) {
		self.send_update(TaskUpdate::Succeeded);
		// Prevent Drop::drop, which will mark the task as failed.
		forget(self);
	}

	/// Mark the task as failed.
	///
	/// Dropping the [`TaskStatusUpdater`] has the same effect.
	pub fn failed(self) {}

	fn send_update(&self, update: TaskUpdate) {
		self.worker_status_updater.send_update(WorkerUpdate::Task {
			task_id: self.task_id,
			update,
		});
	}
}

impl<'a> Drop for TaskStatusUpdater<'a> {
	fn drop(&mut self) {
		self.send_update(TaskUpdate::Failed);
	}
}
