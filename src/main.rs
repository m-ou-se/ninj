extern crate pile;

use std::convert::AsRef;

mod spec;
use spec::read;

fn main() {
	match read("build.ninja".as_ref()) {
		Ok(result) => println!("spec: {:#?}", result),
		Err(error) => {
			println!("{:#?}", error);
			println!("{}", error);
		}
	}
}
