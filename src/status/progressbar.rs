use std::fmt::Write;

pub struct ProgressBar<'a> {
	pub progress: f64,
	pub width: usize,
	pub ascii: bool,
	pub label: &'a str,
}

impl<'a> std::fmt::Display for ProgressBar<'a> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		let blocks = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
		let ticks = (self.progress * self.width as f64 * blocks.len() as f64) as usize;
		let label_pos = self.width.saturating_sub(self.label.len()) / 2;
		f.write_str("\x1b[32m")?;
		let mut i = 0;
		while i < self.width {
			if i >= label_pos && i < label_pos + self.label.len() {
				f.write_str(if i < ticks / blocks.len() {
					"\x1b[32;7m"
				} else {
					"\x1b[m"
				})?;
				f.write_str(&self.label[i - label_pos..][..1])?;
				f.write_str("\x1b[27;32m")?;
			} else {
				f.write_char(if i < ticks / blocks.len() {
					if self.ascii {
						'='
					} else {
						'█'
					}
				} else if i == ticks / blocks.len() {
					if self.ascii {
						'>'
					} else {
						blocks[ticks % 8]
					}
				} else {
					' '
				})?;
			}
			i += 1;
		}
		f.write_str("\x1b[m")?;
		Ok(())
	}
}
