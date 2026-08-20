// Ring buffer wrapper for real-time audio data transfer.
//
// Uses rtrb (Real-Time Ring Buffer) which is a lock-free SPSC queue
// designed for audio. The worker thread is the producer; the audio
// callback is the consumer.
//
// For the initial implementation, decks read directly from their decoded
// buffer in memory. The ring buffer is available for streaming decode
// when full-length tracks need to be loaded without blocking.

pub use rtrb::{Producer, Consumer, RingBuffer};

/// A consumer end of a ring buffer, wrapped for the audio module.
pub struct RingBufferConsumer {
    consumer: Consumer<f32>,
}

/// A producer end of a ring buffer, wrapped for the audio module.
pub struct RingBufferProducer {
    producer: Producer<f32>,
}

impl RingBufferConsumer {
    pub fn new(consumer: Consumer<f32>) -> Self {
        Self { consumer }
    }

    /// Pop one sample. Returns None if the buffer is empty.
    #[inline]
    pub fn pop(&mut self) -> Option<f32> {
        self.consumer.pop().ok()
    }

    /// Number of samples available to read.
    pub fn available(&self) -> usize {
        self.consumer.slots()
    }
}

impl RingBufferProducer {
    pub fn new(producer: Producer<f32>) -> Self {
        Self { producer }
    }

    /// Push one sample. Returns false if the buffer is full.
    pub fn push(&mut self, sample: f32) -> bool {
        self.producer.push(sample).is_ok()
    }

    /// Number of slots available for writing.
    pub fn slots(&self) -> usize {
        self.producer.slots()
    }
}

/// Create a paired ring buffer with the given capacity.
pub fn create_ring_buffer(capacity: usize) -> (RingBufferProducer, RingBufferConsumer) {
    let (producer, consumer) = RingBuffer::<f32>::new(capacity);
    (
        RingBufferProducer::new(producer),
        RingBufferConsumer::new(consumer),
    )
}
