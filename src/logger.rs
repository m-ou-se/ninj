use log::{Metadata, Record};

pub struct Logger;

impl log::Log for Logger {
	fn enabled(&self, _: &Metadata) -> bool {
		true
	}

	fn log(&self, record: &Record) {
		eprintln!(
			"[{}] {}: {}",
			record.level(),
			record.target(),
			record.args()
		);
	}

	fn flush(&self) {}
}
