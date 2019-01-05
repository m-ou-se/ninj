use std::convert::AsRef;

use ninj::spec::read;

fn main() {
	match read("build.ninja".as_ref()) {
		Ok(result) => println!("spec: {:#?}", result),
		Err(error) => println!("{}", error),
	}
}
