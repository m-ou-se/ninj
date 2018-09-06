extern crate pile;

use std::convert::AsRef;

mod spec;
use spec::read;

fn main() {
	println!("spec: {:#?}", read("build.ninja".as_ref()));
}
