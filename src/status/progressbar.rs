use std::fmt::Write;

pub struct ProgressBar<'a> {
	pub progress: f64,
	pub width: usize,
	pub ascii: bool,
	pub label: &'a str,
}

impl<'a> std::fmt::Display for ProgressBar<'a> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		if self.label.len() > self.width {
			f.write_str(&self.label[..self.width])?;
			return Ok(());
		}

		let fancy_blocks = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];

		let ticks = (self.progress * self.width as f64 * fancy_blocks.len() as f64) as usize;
		let label_pos = (self.width - self.label.len()) / 2;
		f.write_str("\x1b[32m")?;
		let mut i = 0;
		while i < self.width {
			if i == label_pos && !self.label.is_empty() {
				f.write_str("\x1b[m")?;
				f.write_str(self.label)?;
				f.write_str("\x1b[32m")?;
				i += self.label.len();
			} else {
				f.write_char(if i < ticks / fancy_blocks.len() {
					if self.ascii {
						'='
					} else if i + 1 == label_pos {
						fancy_blocks[5]
					} else {
						fancy_blocks[7]
					}
				} else if i == ticks / fancy_blocks.len() {
					if self.ascii {
						'>'
					} else {
						fancy_blocks[ticks % 8]
					}
				} else {
					' '
				})?;
				i += 1;
			}
		}
		f.write_str("\x1b[m")?;
		Ok(())
	}
}
