use std::{collections::VecDeque, sync::{Arc, Mutex}};
use super::start_streaming;


#[derive(Debug, Clone)]
pub struct StreamPlayer {
    pub frame_buffer_size: usize,
    frames: FrameQueue

}

impl StreamPlayer {
    pub fn new() -> Self {
        StreamPlayer {
            frame_buffer_size: 0,
            frames: FrameQueue::new(),
        }
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        self.frames.next_frame()
    }

    pub fn play(&self, stream_uri: &str) {
        start_streaming(stream_uri, (1280, 720), self.frames.clone())
    }
}


#[derive(Debug, Clone)]
pub struct FrameQueue {
    capacity: usize,
    frames: Arc<Mutex<VecDeque<Vec<u8>>>>,
}

impl FrameQueue {
    pub fn new() -> Self {
        let capacity = 3;
        FrameQueue {
            capacity,
            frames: Arc::new(Mutex::new(VecDeque::with_capacity(capacity)))
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        FrameQueue {
            capacity,
            frames: Arc::new(Mutex::new(VecDeque::with_capacity(capacity)))
        }
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        self.frames.lock().unwrap().pop_front()
    }

    pub fn put_new_frame(&mut self, frame: Vec<u8>) {
        let mut frames = self.frames.lock().unwrap();
        if frames.len() >= self.capacity {
            frames.pop_front();
        }
        frames.push_back(frame);
    }
}
