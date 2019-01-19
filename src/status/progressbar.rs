use std::fmt::Write;

pub struct ProgressBar<'a> {
	pub progress: f64,
	pub width: usize,
	pub ascii: bool,
	pub label: &'a str,
}

impl<'a> std::fmt::Display for ProgressBar<'a> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		let ticks = (self.progress * self.width as f64 * 8.0) as usize;
		let label_pos = (self.width - self.label.len()) / 2;
		let mut i = 0;
		while i < self.width {
			if i == label_pos && !self.label.is_empty() {
				f.write_str(self.label)?;
				i += self.label.len();
			} else {
				f.write_char(if i < ticks / 8 {
					if self.ascii { '=' } else if i + 1 == label_pos { '▉' } else { '█' }
				} else if i == ticks / 8 {
					if self.ascii { '>' } else { [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'][ticks % 8] }
				} else {
					' '
				})?;
				i += 1;
			}
		}
		Ok(())
	}
}
