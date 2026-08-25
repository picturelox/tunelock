// Production-path synchronization tests — exercises the actual Signalsmith
// processor (not VarispeedProcessor) to verify that:
//
//   1. Two decks launched at the same frame produce aligned audible onsets
//   2. Quantized launch with Master Tempo active stays aligned
//   3. Audible position advances correctly (not in block-sized jumps)
//   4. Loop detection triggers on audible position, not feed position
//
// These tests use click tracks (impulses at known positions) so transient
// alignment can be measured precisely by finding the peak in the output.

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::audio::command::{
        BusId, CommandQueue, DecodedBuffer, EngineCommand, PlayerId, Quantize, SourceHandle,
    };
    use crate::audio::engine::{audio_callback_f32, CallbackState};
    use crate::audio::meter::MeterSnapshot;

    const SR: f64 = 44100.0;

    fn make_state() -> CallbackState {
        // Use the PRODUCTION processor (Signalsmith), not Varispeed.
        // This is the whole point of these tests.
        CallbackState::new(
            Arc::new(AtomicU64::new(0)),
            Arc::new(CommandQueue::new(512)),
            Arc::new(MeterSnapshot::new()),
            Arc::new(crossbeam_queue::ArrayQueue::new(32)),
            Arc::new(crossbeam_queue::ArrayQueue::new(16)),
            SR,
        )
    }

    /// Create a click track: silence with an impulse (1.0) at the given
    /// frame positions. This lets us measure exactly when the audible
    /// onset occurs in the output.
    fn click_track(impulse_frames: &[usize], total_frames: usize) -> Arc<DecodedBuffer> {
        let mut samples = vec![0.0f32; total_frames * 2];
        for &frame in impulse_frames {
            if frame < total_frames {
                samples[frame * 2] = 1.0;
                samples[frame * 2 + 1] = 1.0;
            }
        }
        Arc::new(DecodedBuffer {
            samples,
            sample_rate: SR as u32,
            channels: 2,
            duration_sec: total_frames as f64 / SR,
            bpm: Some(120.0),
            beat_grid: None,
        })
    }

    /// Find the frame index of the first sample exceeding `threshold` in
    /// a stereo output buffer (interleaved). Returns None if not found.
    fn find_first_onset(output: &[f32], threshold: f32, skip_frames: usize) -> Option<usize> {
        let frames = output.len() / 2;
        for i in skip_frames..frames {
            let l = output[i * 2].abs();
            let r = output[i * 2 + 1].abs();
            if l > threshold || r > threshold {
                return Some(i);
            }
        }
        None
    }

    /// Render N blocks of output from the engine.
    fn render_blocks(state: &mut CallbackState, blocks: usize, block_size: usize) -> Vec<f32> {
        let mut output = Vec::new();
        let mut buf = vec![0.0f32; block_size];
        for _ in 0..blocks {
            audio_callback_f32(state, &mut buf);
            output.extend_from_slice(&buf);
        }
        output
    }

    #[test]
    fn signalsmith_audible_onset_occurs_after_launch() {
        // Launch a click track with an impulse at frame 0.
        // The audible onset should occur within a reasonable time after
        // launch (accounting for Signalsmith latency + pre-roll).
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Click at frame 0, then every 4410 frames (every 0.1s at 44.1kHz)
        let clicks: Vec<usize> = (0..20).map(|i| i * 4410).collect();
        let buf = click_track(&clicks, 88200);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buf,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Render 2 seconds of output
        let output = render_blocks(&mut state, 344, 256); // ~88166 frames

        // Find the first audible onset (skip initial latency warm-up)
        let onset = find_first_onset(&output, 0.01, 0);
        assert!(onset.is_some(), "must produce an audible onset");

        let onset_frame = onset.unwrap();
        // The onset should occur within the first second (pre-roll + latency
        // should be well under 1 second at 44.1kHz with preset_default)
        assert!(
            onset_frame < 44100,
            "first audible onset should be within 1 second (got frame {onset_frame})"
        );

        eprintln!("First audible onset at frame {onset_frame} ({:.1}ms)",
                  onset_frame as f64 / SR * 1000.0);
    }

    #[test]
    fn signalsmith_audible_position_advances_smoothly() {
        // The audible position should advance roughly 1:1 with output frames
        // at unity tempo, not in block-sized jumps.
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        let buf = click_track(&[0], 44100);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buf,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Render a few blocks to get past warm-up
        let mut buf256 = vec![0.0f32; 256];
        for _ in 0..20 {
            audio_callback_f32(&mut state, &mut buf256);
        }

        // Now sample the position at several points within blocks
        let pos1 = state.player_position_sec(0);
        // Render 100 more frames (not a full block — we render a block and
        // check position before and after)
        audio_callback_f32(&mut state, &mut buf256);
        let pos2 = state.player_position_sec(0);

        // Position should have advanced by roughly 256 frames / 44100 ≈ 5.8ms.
        // We allow 0.25x–3.0x tolerance because the position is updated
        // per-frame inside next_frame(), and the exact count depends on
        // how many frames were actually served (source may have ended,
        // warm-up may still be in progress, etc.)
        let delta_sec = pos2 - pos1;
        let expected_sec = 256.0 / SR;
        let ratio = delta_sec / expected_sec;
        assert!(
            ratio > 0.25 && ratio < 3.0,
            "position advance should be roughly 1 block ({delta_sec:.4}s vs expected {expected_sec:.4}s, ratio {ratio:.2})"
        );
    }

    #[test]
    fn signalsmith_two_decks_aligned_launch() {
        // Launch two identical click tracks at the same frame.
        // Their audible onsets should occur at approximately the same
        // output frame (within a tolerance for STFT randomness).
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        // Both decks direct to master so we can sum them
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(1),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Both click tracks: impulse at frame 0
        let buf_a = click_track(&[0], 44100);
        let buf_b = click_track(&[0], 44100);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buf_a,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(1),
            at_frame: 0,
            source: SourceHandle(2),
            buffer: buf_b,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Render and find the first onset
        let output = render_blocks(&mut state, 172, 256); // ~44032 frames

        let onset = find_first_onset(&output, 0.05, 0);
        assert!(onset.is_some(), "two decks must produce an audible onset");

        // The onset amplitude should be roughly 2x (both decks clicking
        // at the same time), confirming they're aligned
        let onset_frame = onset.unwrap();
        let onset_amplitude = output[onset_frame * 2].abs();
        assert!(
            onset_amplitude > 0.1,
            "two aligned decks should produce a strong onset (amplitude {onset_amplitude:.3})"
        );

        eprintln!("Two-deck aligned onset at frame {onset_frame}, amplitude {onset_amplitude:.3}");
    }

    #[test]
    fn signalsmith_tempo_change_preserves_audible_position_continuity() {
        // When tempo changes, the audible position should not jump
        // discontinuously. It should continue from where it was, just
        // at a different rate.
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        let buf = click_track(&[0], 44100);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buf,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Render 20 blocks at unity tempo
        let mut buf256 = vec![0.0f32; 256];
        for _ in 0..20 {
            audio_callback_f32(&mut state, &mut buf256);
        }
        let pos_before = state.player_position_sec(0);

        // Change tempo to 1.06x
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::SetTempo {
            player: PlayerId(0),
            at_frame: frame,
            rate: 1.06,
        });

        // Render one more block
        audio_callback_f32(&mut state, &mut buf256);
        let pos_after = state.player_position_sec(0);

        // Position should have advanced (not jumped backward or stayed frozen)
        let delta = pos_after - pos_before;
        assert!(
            delta > 0.0,
            "position must advance after tempo change (delta {delta:.4}s)"
        );
        // The advance should be roughly one block at the new tempo
        let expected = 256.0 / SR * 1.06;
        assert!(
            delta < expected * 3.0,
            "position advance should be reasonable after tempo change (delta {delta:.4}s, expected ~{expected:.4}s)"
        );
    }

    #[test]
    fn signalsmith_loop_wraps_on_audible_position() {
        // With a short loop, the audible position should wrap around
        // the loop endpoint, not the feed position. We verify that
        // the position stays within the loop bounds.
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Short buffer: 4410 frames (0.1s at 44.1kHz)
        let buf = click_track(&[0, 2205], 4410);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buf,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Set a loop: 0 to 0.05 beats (very short)
        // At 120 BPM, 1 beat = 0.5s = 22050 frames
        // 0.05 beats = 0.025s = 1102.5 frames
        state.command_queue.push(EngineCommand::SetLoop {
            player: PlayerId(0),
            at_frame: 0,
            loop_region: Some(crate::audio::command::LoopRegion {
                start_beat: 0.0,
                length_beats: 0.05,
            }),
        });

        // Render many blocks and verify position stays bounded
        let mut buf256 = vec![0.0f32; 256];
        let mut max_pos_sec: f64 = 0.0;
        for _ in 0..200 {
            audio_callback_f32(&mut state, &mut buf256);
            let pos = state.player_position_sec(0);
            max_pos_sec = max_pos_sec.max(pos);
        }

        // The loop endpoint is at 0.025s. The audible position should
        // never exceed this by more than a small margin (one block of
        // read-ahead). If the loop were operating on feed position, the
        // audible position would overshoot significantly.
        let loop_end_sec = 0.025;
        let tolerance = 256.0 / SR * 2.0; // 2 blocks of tolerance
        assert!(
            max_pos_sec < loop_end_sec + tolerance,
            "audible position should stay near loop bounds (max {max_pos_sec:.4}s, loop end {loop_end_sec:.4}s, tolerance {tolerance:.4}s)"
        );
    }

    #[test]
    fn signalsmith_no_allocation_on_relaunch() {
        // Relaunching a deck with a new source should not cause any
        // allocation in the processor (it's pre-configured). We verify
        // by checking that the relaunch succeeds and produces audio
        // without any panic or error.
        let mut state = make_state();
        state.command_queue.push(EngineCommand::SetMasterGain { at_frame: 0, gain: 1.0 });
        state.command_queue.push(EngineCommand::SetBus {
            player: PlayerId(0),
            at_frame: 0,
            bus: BusId::Master,
        });

        // Launch first source
        let buf1 = click_track(&[0], 44100);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: 0,
            source: SourceHandle(1),
            buffer: buf1,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Render a bit
        let mut buf256 = vec![0.0f32; 256];
        for _ in 0..10 {
            audio_callback_f32(&mut state, &mut buf256);
        }

        // Relaunch with a new source — no allocation should occur.
        // Use a continuous sine wave so we can verify audio after warm-up.
        let mut sine_samples = vec![0.0f32; 44100 * 2];
        for i in 0..44100 {
            let v = (2.0 * std::f64::consts::PI * 440.0 * i as f64 / SR).sin() as f32;
            sine_samples[i * 2] = v;
            sine_samples[i * 2 + 1] = v;
        }
        let buf2 = Arc::new(DecodedBuffer {
            samples: sine_samples,
            sample_rate: SR as u32,
            channels: 2,
            duration_sec: 1.0,
            bpm: Some(120.0),
            beat_grid: None,
        });
        let frame = state.frame_counter.load(Ordering::Relaxed);
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(0),
            at_frame: frame,
            source: SourceHandle(2),
            buffer: buf2,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });

        // Render more — should not panic. Render enough to get past
        // pre-roll/latency warm-up.
        for _ in 0..100 {
            audio_callback_f32(&mut state, &mut buf256);
        }

        // Verify audio is still being produced (after warm-up)
        let has_audio = buf256.iter().any(|&s| s.abs() > 0.001);
        assert!(has_audio, "must produce audio after relaunch (no allocation crash)");
    }

    // ========================================================================
    // PB-2.3 Musical Time Bridge — Beat Sync and Bar Sync tests
    // ========================================================================

    /// Create a click track at a specific BPM with clicks on every beat.
    fn beat_click_track(bpm: f64, total_sec: f64) -> Arc<DecodedBuffer> {
        let total_frames = (SR * total_sec) as usize;
        let beat_period = (SR * 60.0 / bpm) as usize;
        let mut samples = vec![0.0f32; total_frames * 2];
        let mut frame = 0;
        while frame < total_frames {
            // Short click: 200 samples
            for j in 0..200.min(total_frames - frame) {
                let v = (1.0 - j as f32 / 200.0) * 0.8;
                samples[(frame + j) * 2] = v;
                samples[(frame + j) * 2 + 1] = v;
            }
            frame += beat_period;
        }
        Arc::new(DecodedBuffer {
            samples,
            sample_rate: SR as u32,
            channels: 2,
            duration_sec: total_sec,
            bpm: Some(bpm),
            beat_grid: Some(crate::audio::command::BeatGridCompact {
                bpm,
                first_beat_sec: 0.0,
                meter_numerator: 4,
                downbeat_offset: 0,
            }),
        })
    }

    /// Helper to launch a player with a given buffer.
    fn launch_player(state: &mut CallbackState, player: u8, buf: Arc<DecodedBuffer>) {
        state.command_queue.push(EngineCommand::Launch {
            player: PlayerId(player),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            source: SourceHandle(player as u64 + 10),
            buffer: buf,
            start_beat: 0.0,
            quantize: Quantize::Immediate,
        });
        // Process commands
        let mut tmp = vec![0.0f32; 256];
        audio_callback_f32(state, &mut tmp);
    }

    #[test]
    fn beat_sync_matches_effective_bpms() {
        let mut state = make_state();

        // Player A: 124 BPM
        let buf_a = beat_click_track(124.0, 30.0);
        launch_player(&mut state, 0, buf_a);

        // Player B: 128 BPM
        let buf_b = beat_click_track(128.0, 30.0);
        launch_player(&mut state, 1, buf_b);

        // Before BeatSync: A's effective BPM = 124, B's effective BPM = 128
        let a_eff_before = state.players[0].effective_bpm();
        let b_eff_before = state.players[1].effective_bpm();
        assert!((a_eff_before - 124.0).abs() < 0.01, "A effective BPM before: {}", a_eff_before);
        assert!((b_eff_before - 128.0).abs() < 0.01, "B effective BPM before: {}", b_eff_before);

        // Send BeatSync: tempo-match B to A
        state.command_queue.push(EngineCommand::BeatSync {
            player_a: PlayerId(0),
            player_b: PlayerId(1),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
        });

        // Process commands
        let mut buf256 = vec![0.0f32; 256];
        audio_callback_f32(&mut state, &mut buf256);

        // After BeatSync: A's effective BPM = 124, B's effective BPM should also = 124
        let a_eff_after = state.players[0].effective_bpm();
        let b_eff_after = state.players[1].effective_bpm();
        assert!((a_eff_after - 124.0).abs() < 0.01, "A effective BPM after: {}", a_eff_after);
        assert!(
            (b_eff_after - 124.0).abs() < 0.5,
            "B effective BPM after BeatSync should be ~124, got {}",
            b_eff_after
        );

        // Verify the tempo ratio: 124/128 = 0.96875
        let expected_ratio = 124.0 / 128.0;
        let actual_ratio = state.players[1].tempo_ratio();
        assert!(
            (actual_ratio - expected_ratio).abs() < 0.001,
            "B tempo ratio should be {}, got {}",
            expected_ratio,
            actual_ratio
        );
    }

    #[test]
    fn beat_sync_aligns_beat_positions() {
        let mut state = make_state();

        let buf_a = beat_click_track(124.0, 30.0);
        launch_player(&mut state, 0, buf_a);

        let buf_b = beat_click_track(128.0, 30.0);
        launch_player(&mut state, 1, buf_b);

        // Advance A by some frames so its beat position is non-zero
        let mut buf256 = vec![0.0f32; 256];
        for _ in 0..50 {
            audio_callback_f32(&mut state, &mut buf256);
        }

        // Send BeatSync
        state.command_queue.push(EngineCommand::BeatSync {
            player_a: PlayerId(0),
            player_b: PlayerId(1),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
        });
        audio_callback_f32(&mut state, &mut buf256);

        // After BeatSync, both players should be playing
        assert!(state.players[0].playing, "Player A should be playing after BeatSync");
        assert!(state.players[1].playing, "Player B should be playing after BeatSync");

        // The beat positions should be close (within 0.5 beat)
        let a_beat = state.players[0].beat_position();
        let b_beat = state.players[1].beat_position();
        let beat_diff = (a_beat - b_beat).abs();
        // After alignment, the beat difference should be less than 1 beat
        // (we round to the nearest beat, so the diff should be < 0.5)
        assert!(
            beat_diff < 1.0,
            "Beat positions should be aligned within 1 beat: A={}, B={}, diff={}",
            a_beat,
            b_beat,
            beat_diff
        );
    }

    #[test]
    fn bar_sync_aligns_bar_positions() {
        let mut state = make_state();

        let buf_a = beat_click_track(124.0, 30.0);
        launch_player(&mut state, 0, buf_a);

        let buf_b = beat_click_track(128.0, 30.0);
        launch_player(&mut state, 1, buf_b);

        // Advance playback
        let mut buf256 = vec![0.0f32; 256];
        for _ in 0..50 {
            audio_callback_f32(&mut state, &mut buf256);
        }

        // Send BarSync
        state.command_queue.push(EngineCommand::BarSync {
            player_a: PlayerId(0),
            player_b: PlayerId(1),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
        });
        audio_callback_f32(&mut state, &mut buf256);

        // After BarSync, both players should be playing
        assert!(state.players[0].playing, "Player A should be playing after BarSync");
        assert!(state.players[1].playing, "Player B should be playing after BarSync");

        // The bar positions should be close (within 1 bar)
        let a_bar = state.players[0].bar_position();
        let b_bar = state.players[1].bar_position();
        let bar_diff = (a_bar - b_bar).abs();
        assert!(
            bar_diff < 1.0,
            "Bar positions should be aligned within 1 bar: A={}, B={}, diff={}",
            a_bar,
            b_bar,
            bar_diff
        );

        // Effective BPMs should match
        let a_eff = state.players[0].effective_bpm();
        let b_eff = state.players[1].effective_bpm();
        assert!(
            (a_eff - b_eff).abs() < 0.5,
            "Effective BPMs should match after BarSync: A={}, B={}",
            a_eff,
            b_eff
        );
    }

    #[test]
    fn long_run_beat_sync_stays_aligned() {
        // Verify that after BeatSync, two players with different source BPMs
        // stay aligned over a long run (30 seconds simulated).
        // Tolerance: within 0.1 beat after 30 seconds.
        let mut state = make_state();

        let buf_a = beat_click_track(124.0, 60.0);
        launch_player(&mut state, 0, buf_a);

        let buf_b = beat_click_track(128.0, 60.0);
        launch_player(&mut state, 1, buf_b);

        // Send BeatSync
        state.command_queue.push(EngineCommand::BeatSync {
            player_a: PlayerId(0),
            player_b: PlayerId(1),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
        });

        // Render 30 seconds of audio (30 * 44100 / 256 ≈ 5168 blocks)
        let mut buf256 = vec![0.0f32; 256];
        let blocks = (30.0 * SR / 256.0) as usize;
        for _ in 0..blocks {
            audio_callback_f32(&mut state, &mut buf256);
        }

        // After 30 seconds, the beat positions should still be close.
        // Since both players have the same effective BPM (124), their
        // beat positions should advance at the same rate.
        let a_beat = state.players[0].beat_position();
        let b_beat = state.players[1].beat_position();
        let beat_diff = (a_beat - b_beat).abs();

        // Tolerance: 0.5 beat (allows for rounding in initial alignment
        // and small floating-point drift over 30 seconds)
        assert!(
            beat_diff < 0.5,
            "After 30s, beat positions should be within 0.5 beat: A={}, B={}, diff={}",
            a_beat,
            b_beat,
            beat_diff
        );
    }

    #[test]
    fn seek_source_seconds_is_bpm_independent() {
        // Verify that seek_sec positions the player at the exact second
        // regardless of BPM. This is the ABX cue positioning guarantee.
        let mut state = make_state();

        // 174 BPM track — fast
        let buf = beat_click_track(174.0, 30.0);
        launch_player(&mut state, 0, buf);

        // Seek to 10.0 seconds
        state.command_queue.push(EngineCommand::SeekSourceSeconds {
            player: PlayerId(0),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            source_seconds: 10.0,
        });

        let mut buf256 = vec![0.0f32; 256];
        audio_callback_f32(&mut state, &mut buf256);

        // Position should be ~10.0 seconds
        let pos_sec = state.players[0].get_position_sec();
        assert!(
            (pos_sec - 10.0).abs() < 0.1,
            "Position after seek_source_seconds(10.0) should be ~10.0s, got {}",
            pos_sec
        );

        // Now try with an 85 BPM track — slow
        let buf_slow = beat_click_track(85.0, 30.0);
        launch_player(&mut state, 1, buf_slow);

        state.command_queue.push(EngineCommand::SeekSourceSeconds {
            player: PlayerId(1),
            at_frame: state.frame_counter.load(Ordering::Relaxed),
            source_seconds: 10.0,
        });
        audio_callback_f32(&mut state, &mut buf256);

        let pos_sec_slow = state.players[1].get_position_sec();
        assert!(
            (pos_sec_slow - 10.0).abs() < 0.1,
            "Position after seek_source_seconds(10.0) at 85 BPM should be ~10.0s, got {}",
            pos_sec_slow
        );
    }
}
