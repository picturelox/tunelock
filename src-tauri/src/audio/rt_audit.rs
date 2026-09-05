// Realtime audit — verify the audio callback path never allocates or
// deallocates heap memory during normal operation.
//
// This module provides a thread-local counting allocator wrapper that
// tracks Rust global allocator calls. The tests run the callback through
// launch, playback, relaunch, stop, and loop scenarios, asserting that
// no allocations or deallocations occur, including in the first block.
//
// Setup and source destruction are outside the measured callback scopes.
// Native C++ malloc/free (including Signalsmith internals) are not counted.

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::marker::PhantomData;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::audio::command::{
        DecodedBuffer, EngineCommand, Quantize, SourceHandle,
    };
    use crate::audio::engine::{audio_callback_f32, CallbackState};
    use crate::audio::meter::MeterSnapshot;
    use crate::audio::command::{BusId, CommandQueue, PlayerId};

    const SR: f64 = 44100.0;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct AllocationCounts {
        alloc: usize,
        alloc_zeroed: usize,
        realloc: usize,
        dealloc: usize,
    }

    thread_local! {
        static ALLOCATION_COUNTS: Cell<Option<AllocationCounts>> = const { Cell::new(None) };
    }

    struct CountingAllocator;

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    fn record_operation(select: fn(&mut AllocationCounts) -> &mut usize) {
        let _ = ALLOCATION_COUNTS.try_with(|slot| {
            if let Some(mut counts) = slot.get() {
                let counter = select(&mut counts);
                *counter = counter.saturating_add(1);
                slot.set(Some(counts));
            }
        });
    }

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            record_operation(|counts| &mut counts.alloc);
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            record_operation(|counts| &mut counts.alloc_zeroed);
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            record_operation(|counts| &mut counts.realloc);
            unsafe { System.realloc(ptr, layout, new_size) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            record_operation(|counts| &mut counts.dealloc);
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    struct AllocationScope {
        _thread_bound: PhantomData<*mut ()>,
    }

    impl AllocationScope {
        fn begin() -> Self {
            ALLOCATION_COUNTS.with(|slot| {
                assert!(slot.get().is_none(), "allocation scopes must not nest");
                slot.set(Some(AllocationCounts::default()));
            });
            Self { _thread_bound: PhantomData }
        }

        fn finish(self) -> AllocationCounts {
            ALLOCATION_COUNTS.with(|slot| slot.replace(None)).expect("active allocation scope")
        }
    }

    impl Drop for AllocationScope {
        fn drop(&mut self) {
            let _ = ALLOCATION_COUNTS.try_with(|slot| slot.set(None));
        }
    }

    fn measure_allocations(callback: impl FnOnce()) -> AllocationCounts {
        let scope = AllocationScope::begin();
        callback();
        scope.finish()
    }

    #[track_caller]
    fn audited_callback(state: &mut CallbackState, output: &mut [f32]) {
        let counts = measure_allocations(|| audio_callback_f32(state, output, 2));
        assert_eq!(counts, AllocationCounts::default(), "Rust heap activity in audio callback");
    }

    fn exercise_allocator() {
        let layout = Layout::from_size_align(32, 8).unwrap();
        let grown_layout = Layout::from_size_align(64, 8).unwrap();
        unsafe {
            let ptr = std::alloc::alloc(layout);
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            std::hint::black_box(ptr).write(7);
            let grown = std::alloc::realloc(ptr, layout, grown_layout.size());
            if grown.is_null() {
                std::alloc::dealloc(ptr, layout);
                std::alloc::handle_alloc_error(grown_layout);
            }
            std::hint::black_box(grown).write(11);
            std::alloc::dealloc(grown, grown_layout);
            let zeroed = std::alloc::alloc_zeroed(layout);
            if zeroed.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            std::hint::black_box(zeroed.read());
            std::alloc::dealloc(zeroed, layout);
        }
    }

    #[test]
    fn allocator_counts_all_operations_and_excludes_untracked_work() {
        exercise_allocator();
        let counts = measure_allocations(exercise_allocator);
        exercise_allocator();
        assert_eq!(counts, AllocationCounts { alloc: 1, alloc_zeroed: 1, realloc: 1, dealloc: 2 });
        assert_eq!(measure_allocations(|| {}), AllocationCounts::default());
        assert!(ALLOCATION_COUNTS.with(|slot| slot.get().is_none()));
    }

    #[test]
    fn allocator_tracking_is_thread_local() {
        use std::sync::atomic::AtomicBool;

        let ready = AtomicBool::new(false);
        let start = AtomicBool::new(false);
        let done = AtomicBool::new(false);
        std::thread::scope(|threads| {
            let worker = threads.spawn(|| {
                ready.store(true, Ordering::Release);
                while !start.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
                let counts = measure_allocations(|| {
                    for _ in 0..128 {
                        exercise_allocator();
                    }
                });
                done.store(true, Ordering::Release);
                counts
            });
            while !ready.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
            let counts = measure_allocations(|| {
                start.store(true, Ordering::Release);
                while !done.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            });
            let worker_counts = worker.join().unwrap();
            assert_eq!(counts, AllocationCounts::default());
            assert_eq!(worker_counts, AllocationCounts {
                alloc: 128, alloc_zeroed: 128, realloc: 128, dealloc: 256,
            });
        });
    }

    #[test]
    fn allocator_tracking_is_disabled_after_unwind() {
        let result = std::panic::catch_unwind(|| {
            measure_allocations(|| panic!("intentional allocation-scope unwind"));
        });
        assert!(ALLOCATION_COUNTS.with(|slot| slot.get().is_none()));
        assert!(result.is_err());
        drop(result);
        assert_eq!(measure_allocations(|| {}), AllocationCounts::default());
    }

    fn make_state() -> CallbackState {
        CallbackState::new_for_test(
            Arc::new(AtomicU64::new(0)),
            Arc::new(CommandQueue::new(512)),
            Arc::new(MeterSnapshot::new()),
            Arc::new(crossbeam_queue::ArrayQueue::new(32)),
            SR,
        )
    }

    fn constant_buffer(value: f32, frames: usize) -> Arc<DecodedBuffer> {
        let mut samples = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            samples.push(value);
            samples.push(value);
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

    // The test-only global allocator delegates to System and observes only
    // the measured thread, so parallel tests do not contaminate its counts.
    // We audit by running the callback through a full lifecycle
    // and verifying that:
    //   1. No Rust allocations, zeroed allocations, reallocations or frees occur
    //   2. The pending vector doesn't grow beyond its initial capacity
    //   3. The retirement queue catches old buffers (no drops on callback)
    //   4. Repeated blocks and meter report windows remain allocation-free
    //
    // Constant-initialized TLS Cells need no heap allocation or locks.
    // Scoped tracking is disabled before assertions and during unwinding.
    // Construction, command preparation and retirement draining are excluded;
    // the first callback block is measured without any warm-up exemption.

    fn audit_reporting_windows(mut state: CallbackState) {
        let initial_cap = state.pending_capacity_for_test();
        let sizes = [1, 17, 127, 256, 511, 1470, 2048, 2049, 4097];
        let mut output = [0.0f32; 4097 * 2];
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0), at_frame: 0, bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.3, SR as usize * 30),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::SetTempo {
            player: PlayerId(0), at_frame: 0, rate: 1.125,
        });
        state.command_queue.push(EngineCommand::SetPitch {
            player: PlayerId(0), at_frame: 0, semitones: 2.0,
        });
        state.command_queue.push(EngineCommand::SetGain {
            player: PlayerId(0), at_frame: 8193, gain: 0.8, ramp_frames: 128,
        });
        audited_callback(&mut state, &mut output);
        assert!(output.iter().any(|&sample| sample != 0.0), "first block must produce audio");

        for gain in [1.0, 1.5, 1.0] {
            state.command_queue.push(EngineCommand::SetLoudnessMatchGain {
                player: PlayerId(0), gain,
            });
            for _ in 0..16 {
                for frames in sizes {
                    let block = &mut output[..frames * 2];
                    audited_callback(&mut state, block);
                    assert!(block.iter().all(|sample| sample.is_finite()));
                    assert!(block.iter().any(|&sample| sample != 0.0), "playback must remain active");
                }
            }
            assert_eq!(state.players[0].loudness_match_gain(), gain);
            assert!(state.players[0].playing);
        }

        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Pause { player: PlayerId(0), at_frame: frame });
        audited_callback(&mut state, &mut output);
        assert!(output.iter().all(|&sample| sample == 0.0));
        let paused_position = state.player_position_sec(0);
        for _ in 0..4 {
            for frames in sizes {
                let block = &mut output[..frames * 2];
                audited_callback(&mut state, block);
                assert!(block.iter().all(|&sample| sample == 0.0));
            }
        }
        assert!(!state.players[0].playing);
        assert_eq!(state.player_position_sec(0), paused_position);

        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Resume { player: PlayerId(0), at_frame: frame });
        audited_callback(&mut state, &mut output);
        assert!(state.players[0].playing);
        assert!(output.iter().any(|&sample| sample != 0.0));

        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: frame,
            source: SourceHandle(2),
            buffer: constant_buffer(0.5, SR as usize * 30),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        audited_callback(&mut state, &mut output);
        assert!(output.iter().any(|&sample| sample != 0.0));
        let mut retired = 0;
        while let Some(source) = state.retired_sources_pop_for_test() {
            drop(source);
            retired += 1;
        }
        assert_eq!(retired, 2, "both old source owners must be retired outside the callback");

        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Stop { player: PlayerId(0), at_frame: frame });
        audited_callback(&mut state, &mut output);
        assert!(output.iter().all(|&sample| sample == 0.0));
        for _ in 0..4 {
            for frames in sizes {
                let block = &mut output[..frames * 2];
                audited_callback(&mut state, block);
                assert!(block.iter().all(|&sample| sample == 0.0));
            }
        }
        assert!(!state.players[0].playing);
        assert_eq!(state.player_position_sec(0), 0.0);
        assert_eq!(state.pending_capacity_for_test(), initial_cap);
    }

    #[test]
    fn callback_reporting_windows_varispeed_has_no_rust_heap_activity() {
        let state = make_state();
        assert_eq!(state.players[0].processor_mode(), crate::audio::timepitch::ProcessorMode::Varispeed);
        audit_reporting_windows(state);
    }

    #[test]
    fn callback_reporting_windows_signalsmith_has_no_rust_heap_activity() {
        let state = CallbackState::new(
            Arc::new(AtomicU64::new(0)),
            Arc::new(CommandQueue::new(512)),
            Arc::new(MeterSnapshot::new()),
            Arc::new(crossbeam_queue::ArrayQueue::new(32)),
            Arc::new(crossbeam_queue::ArrayQueue::new(16)),
            SR,
        );
        assert_eq!(state.players[0].processor_mode(), crate::audio::timepitch::ProcessorMode::Signalsmith);
        audit_reporting_windows(state);
    }

    #[test]
    fn callback_lifecycle_has_no_rust_heap_activity_and_stable_capacity() {
        let mut state = make_state();
        let initial_pending_cap = state.pending_capacity_for_test();

        // Setup: master unity, player 0 direct-to-master
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Block 1: launch + play
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.3, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        let mut out = vec![0.0f32; 512];
        audited_callback(&mut state, &mut out);
        assert!(!out.iter().all(|&s| s == 0.0), "block 1 must produce audio");

        // Blocks 2-10: steady playback, no new commands
        for _ in 0..9 {
            audited_callback(&mut state, &mut out);
        }

        // Pending vector must not have grown
        assert_eq!(
            state.pending_capacity_for_test(),
            initial_pending_cap,
            "pending vector capacity must not grow during steady playback"
        );

        // Block 11: relaunch with new source (retirement path)
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: frame,
            source: SourceHandle(2),
            buffer: constant_buffer(0.5, 44100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        audited_callback(&mut state, &mut out);

        // Block 12-15: steady playback with new source
        for _ in 0..4 {
            audited_callback(&mut state, &mut out);
        }

        // Block 16: stop
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Stop { player: PlayerId(0), at_frame: frame });
        audited_callback(&mut state, &mut out);
        assert!(out.iter().all(|&s| s == 0.0), "stopped player must be silent");

        // Pending capacity still unchanged
        assert_eq!(
            state.pending_capacity_for_test(),
            initial_pending_cap,
            "pending vector capacity must not grow through full lifecycle"
        );

        // Retirement queue should have caught the old buffer
        let mut retired = 0;
        while state.retired_sources_pop_for_test().is_some() {
            retired += 1;
        }
        assert!(retired >= 1, "at least one buffer must be retired through the queue");
    }

    #[test]
    fn callback_loop_wrap_has_no_rust_heap_activity() {
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Short buffer: 100 frames at 44100 Hz = ~2.3ms
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: constant_buffer(0.3, 100),
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Set a loop that wraps within the buffer
        state.command_queue.push(EngineCommand::SetLoop {
            player: PlayerId(0),
            at_frame: 0,
            loop_region: Some(crate::audio::command::LoopRegion {
                start_beat: 0.0,
                length_beats: 64.0 * 120.0 / (60.0 * SR),
            }),
        });

        let initial_cap = state.pending_capacity_for_test();
        let mut out = vec![0.0f32; 512];

        // Render many blocks — the loop should wrap repeatedly
        for _ in 0..100 {
            audited_callback(&mut state, &mut out);
            assert!(state.players[0].playing, "loop must keep playing");
            assert!(out.iter().any(|&sample| sample != 0.0));
        }

        assert_eq!(
            state.pending_capacity_for_test(),
            initial_cap,
            "loop wrapping must not grow the pending vector"
        );
    }

    #[test]
    fn retirement_queue_overflow_holds_source_not_drops() {
        // Verify that when the retirement queue is full, old sources are
        // held in the player's fixed overflow slots rather than being
        // dropped on the realtime thread. We fill the queue and overflow,
        // then drain and relaunch, verifying the old sources are not lost.
        let mut state = make_state();

        // Use the bounded queue so we can fill it easily
        // (32 queued Arcs plus 8 player overflow slots hold the 40 Arcs
        // retired by 20 relaunches, before any engine-thread draining)

        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Do many rapid relaunches to fill the retirement queue
        for i in 0..21 {
            let buf = constant_buffer(0.3, 4410);
            let frame = state.frame_counter.load(Ordering::Relaxed);
            state.command_queue.push(EngineCommand::Launch {
                player: PlayerId(0),
                at_frame: frame,
                source: SourceHandle(i + 1),
                buffer: buf,
                start_beat: 0.0,
                quantize: Quantize::Immediate,
            });
            // Process one block to trigger the launch
            let mut out = vec![0.0f32; 256];
            audited_callback(&mut state, &mut out);
        }

        // The retirement queue and all player overflow slots are full.
        // The measured callbacks must not allocate or deallocate;
        // overflowing all bounded queues is outside this test's scope.
        // The player holds overflow sources until the queue is drained.

        // Now drain the queue
        let mut drained = 0;
        while state.retired_sources_pop_for_test().is_some() {
            drained += 1;
        }
        // We should have drained sources up to the queue capacity (32).
        // The remaining 8 Arcs are held in the player's overflow slots.
        // The allocator assertions exclude source destruction while draining.
        assert_eq!(drained, 32, "retirement queue must be full");

        // After draining, do one more relaunch and verify it works
        let buf = constant_buffer(0.5, 4410);
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: frame,
            source: SourceHandle(22),
            buffer: buf,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        let mut out = vec![0.0f32; 256];
        audited_callback(&mut state, &mut out);
        // All 8 overflow Arcs and the 2 newly retired Arcs are now queued.
        // Destruction happens here, outside the measured callback scope.
        let mut remaining = 0;
        while state.retired_sources_pop_for_test().is_some() {
            remaining += 1;
        }
        assert_eq!(remaining, 10, "all overflow owners must survive until draining");
    }
}
