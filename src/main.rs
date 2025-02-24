#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(non_snake_case)]

mod stream;
mod system;
mod vid;

use std::collections::HashMap;
use std::error::Error;

fn main() {
    let socket_path = "/tmp/video-processor.sock";

    stream::start_streaming(socket_path).expect("Error running streaming service");
    // TODO: Handle error appropriately
}
