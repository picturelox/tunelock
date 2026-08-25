// PB-2 Performance harness — measures maximum callback execution time
// versus the audio callback deadline for 2 and 4 active decks.
//
// The reviewer specifically asked for this:
// > I would measure maximum callback execution time versus audio callback
// > deadline. That's what tells us whether playback is rock-solid.
//
// At 48 kHz / 128 samples, the callback budget is ~2.67ms. A single
// periodic processing spike matters far more than an average reading.
//
// IMPORTANT: These tests are #[ignore] by default because they require
// release mode for accurate timing. Debug builds are 10-20x slower.
// Run with:
//   cargo test --release --features perf -- --ignored
//
// Or individually:
//   cargo test --release perf_2_decks_48k_256 -- --ignored

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    use crate::audio::command::{
        BusId, CommandQueue, DecodedBuffer, EngineCommand, PlayerId, Quantize, SourceHandle,
    };
    use crate::audio::engine::{audio_callback_f32, CallbackState};
    use crate::audio::meter::MeterSnapshot;

    const SR: f64 = 48000.0;

    fn make_state() -> CallbackState {
        // Use production processor (Signalsmith) for realistic timing
        CallbackState::new(
            Arc::new(AtomicU64::new(0)),
            Arc::new(CommandQueue::new(512)),
            Arc::new(MeterSnapshot::new()),
            Arc::new(crossbeam_queue::ArrayQueue::new(128)),
            SR,
        )
    }

    fn sine_buffer(freq: f64, frames: usize) -> Arc<DecodedBuffer> {
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f64 / SR;
            let v = (2.0 * std::f64::consts::PI * freq * t).sin() as f32;
            samples.push(v);
            samples.push(v);
        }
        Arc::new(DecodedBuffer {
            samples,
            sample_rate: SR as u32,
            channels: 2,
            duration_sec: frames as f64 / SR,
            bpm: Some(120.0),
            beat_grid: None,
        })
    }

    /// Launch a deck with a sine wave source.
    fn launch_deck(state: &mut CallbackState, player: usize, freq: f64, tempo: f64) {
        let buf = sine_buffer(freq, SR as usize * 10); // 10 seconds
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(player as u8),
            at_frame: frame,
            bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::SetTempo {
            player: PlayerId(player as u8),
            at_frame: frame,
            rate: tempo as f32,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(player as u8),
            at_frame: frame,
            source: SourceHandle(player as u64 + 1),
            buffer: buf,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
    }

    /// Measure callback execution times over a number of blocks.
    /// Returns (max_us, avg_us, p99_us).
    fn measure_callback_times(
        state: &mut CallbackState,
        block_size: usize,
        num_blocks: usize,
    ) -> (f64, f64, f64) {
        let mut buf = vec![0.0f32; block_size];
        let mut times_us = Vec::with_capacity(num_blocks);

        // Skip warm-up (first 100 blocks to get past initial ring-filling
        // and pre-roll spikes). We want to measure steady-state behavior.
        for _ in 0..100 {
            audio_callback_f32(state, &mut buf);
        }

        // Measure
        for _ in 0..num_blocks {
            let start = Instant::now();
            audio_callback_f32(state, &mut buf);
            let elapsed = start.elapsed().as_secs_f64() * 1e6;
            times_us.push(elapsed);
        }

        let max_us = times_us.iter().cloned().fold(0.0f64, f64::max);
        let avg_us = times_us.iter().sum::<f64>() / times_us.len() as f64;
        // p99: sort and take the 99th percentile
        times_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p99_idx = (times_us.len() as f64 * 0.99) as usize;
        let p99_us = times_us[p99_idx.min(times_us.len() - 1)];

        (max_us, avg_us, p99_us)
    }

    #[test]
    #[ignore = "run with: cargo test --release -- --ignored"]
    fn perf_2_decks_48k_128_budget_2667us() {
        // At 48 kHz / 128 samples, the callback budget is:
        // 128 / 48000 = 2.667ms = 2667µs
        let budget_us = 128.0 / SR * 1e6;
        let block_size = 128;

        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });

        // Launch 2 decks with different tempos
        launch_deck(&mut state, 0, 220.0, 1.06);  // +6% tempo
        launch_deck(&mut state, 1, 330.0, 0.94);  // -6% tempo

        let (max_us, avg_us, p99_us) = measure_callback_times(&mut state, block_size, 500);

        eprintln!(
            "2 decks @ 48k/128: max={max_us:.0}µs, avg={avg_us:.0}µs, p99={p99_us:.0}µs, budget={budget_us:.0}µs"
        );

        // Max callback time must be under the budget
        assert!(
            max_us < budget_us,
            "2-deck max callback {max_us:.0}µs must be under budget {budget_us:.0}µs"
        );
    }

    #[test]
    #[ignore = "run with: cargo test --release -- --ignored"]
    fn perf_4_decks_48k_256_budget_5333us() {
        // Future architecture: 4 active decks. At 256 samples the budget
        // is 5333µs, which gives enough headroom for 4 Signalsmith instances.
        // 4 decks at 128 samples is a known limitation — periodic ring
        // refill spikes can exceed the 2667µs budget. Use 256+ for 4 decks.
        let budget_us = 256.0 / SR * 1e6;
        let block_size = 256;

        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });

        // Launch 4 decks with different tempos and pitches
        launch_deck(&mut state, 0, 220.0, 1.06);
        launch_deck(&mut state, 1, 330.0, 0.94);
        launch_deck(&mut state, 2, 440.0, 1.02);
        launch_deck(&mut state, 3, 550.0, 0.98);

        let (max_us, avg_us, p99_us) = measure_callback_times(&mut state, block_size, 500);

        eprintln!(
            "4 decks @ 48k/256: max={max_us:.0}µs, avg={avg_us:.0}µs, p99={p99_us:.0}µs, budget={budget_us:.0}µs"
        );

        assert!(
            max_us < budget_us,
            "4-deck max callback {max_us:.0}µs must be under budget {budget_us:.0}µs"
        );
    }

    #[test]
    #[ignore = "run with: cargo test --release -- --ignored"]
    fn perf_2_decks_48k_256_budget_5333us() {
        // At 48 kHz / 256 samples, the callback budget is 5333µs.
        // This is a more comfortable operating point.
        let budget_us = 256.0 / SR * 1e6;
        let block_size = 256;

        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });

        launch_deck(&mut state, 0, 220.0, 1.10);  // +10% tempo
        launch_deck(&mut state, 1, 330.0, 0.90);  // -10% tempo

        let (max_us, avg_us, p99_us) = measure_callback_times(&mut state, block_size, 500);

        eprintln!(
            "2 decks @ 48k/256: max={max_us:.0}µs, avg={avg_us:.0}µs, p99={p99_us:.0}µs, budget={budget_us:.0}µs"
        );

        assert!(
            max_us < budget_us,
            "2-deck max callback {max_us:.0}µs must be under budget {budget_us:.0}µs"
        );
    }

    #[test]
    #[ignore = "run with: cargo test --release -- --ignored"]
    fn perf_2_decks_441k_256_budget_5805us() {
        // At 44.1 kHz / 256 samples, the callback budget is 5805µs.
        let sr = 44100.0;
        let budget_us = 256.0 / sr * 1e6;
        let block_size = 256;

        let mut state = CallbackState::new(
            Arc::new(AtomicU64::new(0)),
            Arc::new(CommandQueue::new(512)),
            Arc::new(MeterSnapshot::new()),
            Arc::new(crossbeam_queue::ArrayQueue::new(128)),
            sr,
        );
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });

        // Launch 2 decks
        let buf0 = sine_buffer(220.0, sr as usize * 10);
        let buf1 = sine_buffer(330.0, sr as usize * 10);
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0), at_frame: frame, bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(1), at_frame: frame, bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0), at_frame: frame, source: SourceHandle(1),
            buffer: buf0, start_beat: 0.0, quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(1), at_frame: frame, source: SourceHandle(2),
            buffer: buf1, start_beat: 0.0, quantize: Quantize::Immediate,
        });

        let (max_us, avg_us, p99_us) = measure_callback_times(&mut state, block_size, 500);

        eprintln!(
            "2 decks @ 44.1k/256: max={max_us:.0}µs, avg={avg_us:.0}µs, p99={p99_us:.0}µs, budget={budget_us:.0}µs"
        );

        assert!(
            max_us < budget_us,
            "2-deck max callback {max_us:.0}µs must be under budget {budget_us:.0}µs"
        );
    }
}
