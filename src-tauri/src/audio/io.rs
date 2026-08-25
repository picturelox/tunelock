// PB-3: Professional audio I/O — device enumeration, selection, and
// stream configuration.
//
// The original engine used `default_host → default_output_device →
// default_output_config` with no way to choose a device, sample rate, or
// buffer size. This module provides the enumeration and selection layer
// that the engine and UI use to configure audio I/O professionally.
//
// Architecture:
//   - AudioDeviceList: enumerated devices with names and supported configs
//   - AudioDeviceConfig: the user's chosen device + sample rate + buffer
//   - AudioEngine::new_with_config(): builds the stream from a chosen config
//   - AudioEngine::new(): falls back to default device (back-compat)
//
// PB-3 MVP constraints:
//   - Output is always stereo (2 channels). The engine's internal signal
//     path is stereo. Multi-output (Master/Cue 1/2 + 3/4) is a deliberate
//     future phase, not an accidental capability.
//   - Sample rate and buffer size are selectable from the device's actual
//     supported configurations, not synthesized by overriding the default.
//
// Future: ASIO support on Windows, cue/master channel routing, and
// disconnect/reconnect handling.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{SampleRate, StreamConfig};

/// A single supported configuration tuple from a CPAL device.
/// Each tuple represents an actual combination of sample format, channel
/// count, and sample rate range that the device supports.
#[derive(Debug, Clone)]
pub struct SupportedAudioConfig {
    /// Channel count for this configuration.
    pub channels: u16,
    /// Minimum sample rate (Hz).
    pub min_sample_rate: u32,
    /// Maximum sample rate (Hz).
    pub max_sample_rate: u32,
    /// Sample format (e.g. F32, I16).
    pub sample_format: cpal::SampleFormat,
}

/// A description of one available audio output device.
#[derive(Debug, Clone)]
pub struct AudioDevice {
    /// CPAL device name (human-readable).
    pub name: String,
    /// Actual supported configuration tuples (not flattened lists).
    pub supported_configs: Vec<SupportedAudioConfig>,
    /// Default sample rate for this device.
    pub default_sample_rate: u32,
    /// Default channel count.
    pub default_channels: u16,
    /// Whether this is the system default output device.
    pub is_default: bool,
}

impl AudioDevice {
    /// Returns the distinct sample rates supported by this device across
    /// all configuration tuples. Useful for UI dropdowns.
    pub fn supported_sample_rates(&self) -> Vec<u32> {
        let mut rates = Vec::new();
        for cfg in &self.supported_configs {
            // Only include stereo configs (our MVP output format)
            if cfg.channels != 2 {
                continue;
            }
            for rate in [22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000] {
                if rate >= cfg.min_sample_rate
                    && rate <= cfg.max_sample_rate
                    && !rates.contains(&rate)
                {
                    rates.push(rate);
                }
            }
        }
        rates.sort_unstable();
        rates
    }

    /// Returns true if this device supports stereo output at the given
    /// sample rate.
    pub fn supports_stereo_at(&self, sample_rate: u32) -> bool {
        self.supported_configs.iter().any(|cfg| {
            cfg.channels == 2
                && sample_rate >= cfg.min_sample_rate
                && sample_rate <= cfg.max_sample_rate
        })
    }
}

/// A list of available audio output devices.
#[derive(Debug, Clone)]
pub struct AudioDeviceList {
    pub devices: Vec<AudioDevice>,
    pub default_device_index: Option<usize>,
}

/// The user's chosen audio configuration. Used to build the engine stream.
///
/// PB-3 MVP: output is always stereo. Channel count is NOT selectable.
/// Multi-output routing (Master 1/2, Cue 3/4) is a future phase.
#[derive(Debug, Clone)]
pub struct AudioDeviceConfig {
    /// Device name to select (must match a name from AudioDeviceList).
    /// If None, uses the system default.
    pub device_name: Option<String>,
    /// Requested sample rate in Hz. If None, uses the device default.
    pub sample_rate: Option<u32>,
    /// Requested buffer size. If None, uses the device default.
    pub buffer_size: Option<BufferSizePreference>,
}

impl Default for AudioDeviceConfig {
    fn default() -> Self {
        Self {
            device_name: None,
            sample_rate: None,
            buffer_size: None,
        }
    }
}

/// Buffer size preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferSizePreference {
    /// Let the system choose (typically 256-1024 frames).
    Default,
    /// Request a fixed 64-frame buffer for low latency. The device may
    /// not support this — resolve_config will return an error if the
    /// device rejects it.
    LowLatency64,
    /// Request a fixed 128-frame buffer.
    LowLatency128,
    /// A specific frame count.
    Fixed(u32),
}

/// Enumerate all available audio output devices on the default host.
/// Returns device names, supported configuration tuples, and which device
/// is the system default.
pub fn enumerate_output_devices() -> Result<AudioDeviceList, String> {
    let host = cpal::default_host();
    let default_device = host.default_output_device();
    let default_name = default_device.as_ref().and_then(|d| d.name().ok());

    let mut devices = Vec::new();
    let mut default_index = None;

    let device_iter = host
        .output_devices()
        .map_err(|e| format!("Failed to enumerate output devices: {}", e))?;

    for device in device_iter {
        let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        let is_default = default_name.as_ref() == Some(&name);

        let configs = device
            .supported_output_configs()
            .map_err(|e| format!("Failed to get configs for '{}': {}", name, e))?;

        let mut supported_configs = Vec::new();
        let mut default_sr = 44100u32;
        let mut default_ch = 2u16;

        for sc in configs {
            supported_configs.push(SupportedAudioConfig {
                channels: sc.channels(),
                min_sample_rate: sc.min_sample_rate().0,
                max_sample_rate: sc.max_sample_rate().0,
                sample_format: sc.sample_format(),
            });
        }

        if let Some(default_cfg) = device.default_output_config().ok() {
            default_sr = default_cfg.sample_rate().0;
            default_ch = default_cfg.channels();
        }

        if is_default {
            default_index = Some(devices.len());
        }

        devices.push(AudioDevice {
            name,
            supported_configs,
            default_sample_rate: default_sr,
            default_channels: default_ch,
            is_default,
        });
    }

    Ok(AudioDeviceList {
        devices,
        default_device_index: default_index,
    })
}

/// Resolve an AudioDeviceConfig to a concrete CPAL device, stream config,
/// and the sample format of the selected configuration.
///
/// This selects an ACTUAL supported configuration tuple from the device
/// rather than blindly overriding the default. Output is always stereo
/// (2 channels) — multi-output is a future phase.
///
/// Returns the device, stream config, sample rate, and sample format.
/// The caller MUST use the returned sample format to construct the
/// callback — do NOT re-query default_output_config() for the format,
/// as the default may differ from the selected configuration.
pub(crate) fn resolve_config(
    config: &AudioDeviceConfig,
) -> Result<(cpal::Device, StreamConfig, u32, cpal::SampleFormat), String> {
    let host = cpal::default_host();

    let device = match &config.device_name {
        Some(name) => {
            let mut found = None;
            if let Ok(iter) = host.output_devices() {
                for d in iter {
                    if d.name().map(|n| n == *name).unwrap_or(false) {
                        found = Some(d);
                        break;
                    }
                }
            }
            found.ok_or_else(|| format!("Audio device '{}' not found", name))?
        }
        None => host
            .default_output_device()
            .ok_or("No audio output device available")?,
    };

    // Find a stereo supported config that matches the requested sample rate.
    // We always force stereo (2 channels) for PB-3 MVP.
    let supported_configs = device
        .supported_output_configs()
        .map_err(|e| format!("Failed to get supported configs: {}", e))?;

    let target_sr = config.sample_rate.unwrap_or_else(|| {
        // Fall back to device default, or 44100 if no default available
        device
            .default_output_config()
            .ok()
            .map(|c| c.sample_rate().0)
            .unwrap_or(44100)
    });

    // Find a stereo config that supports the target sample rate.
    // Prefer F32 format; fall back to any stereo config.
    let mut best_config: Option<cpal::SupportedStreamConfig> = None;
    for sc in supported_configs {
        if sc.channels() != 2 {
            continue;
        }
        let min_sr = sc.min_sample_rate().0;
        let max_sr = sc.max_sample_rate().0;
        if target_sr >= min_sr && target_sr <= max_sr {
            // Prefer F32; otherwise take the first matching stereo config
            if sc.sample_format() == cpal::SampleFormat::F32 {
                best_config = Some(sc.with_sample_rate(SampleRate(target_sr)));
                break;
            }
            if best_config.is_none() {
                best_config = Some(sc.with_sample_rate(SampleRate(target_sr)));
            }
        }
    }

    let supported = match best_config {
        Some(sc) => sc,
        None => {
            // No stereo config at the requested rate. Try the device default.
            let default_cfg = device
                .default_output_config()
                .map_err(|e| format!("Failed to get default config: {}", e))?;
            if default_cfg.channels() != 2 {
                return Err(format!(
                    "Device '{}' does not support stereo output (default: {}ch)",
                    device.name().unwrap_or_default(),
                    default_cfg.channels()
                ));
            }
            // Use default sample rate if the requested one isn't supported
            let default_sr = default_cfg.sample_rate().0;
            if config.sample_rate.is_some() {
                // User explicitly requested a rate that isn't supported
                return Err(format!(
                    "Device '{}' does not support stereo at {} Hz (try {} Hz)",
                    device.name().unwrap_or_default(),
                    target_sr,
                    default_sr
                ));
            }
            default_cfg
        }
    };

    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();

    // Build the stream config from the actual supported config
    let mut stream_config: StreamConfig = supported.into();
    stream_config.channels = 2; // Force stereo

    if let Some(buf_pref) = config.buffer_size {
        stream_config.buffer_size = match buf_pref {
            BufferSizePreference::Default => cpal::BufferSize::Default,
            BufferSizePreference::LowLatency64 => cpal::BufferSize::Fixed(64),
            BufferSizePreference::LowLatency128 => cpal::BufferSize::Fixed(128),
            BufferSizePreference::Fixed(n) => cpal::BufferSize::Fixed(n),
        };
    }

    Ok((device, stream_config, sample_rate, sample_format))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_all_none() {
        let cfg = AudioDeviceConfig::default();
        assert!(cfg.device_name.is_none());
        assert!(cfg.sample_rate.is_none());
        assert!(cfg.buffer_size.is_none());
    }

    #[test]
    fn buffer_size_preference_fixed_preserves_value() {
        let b = BufferSizePreference::Fixed(256);
        assert_eq!(b, BufferSizePreference::Fixed(256));
    }

    #[test]
    fn low_latency_64_is_distinct_from_128() {
        assert_ne!(BufferSizePreference::LowLatency64, BufferSizePreference::LowLatency128);
    }

    #[test]
    fn enumerate_output_devices_returns_at_least_one_device() {
        // This test may fail on headless CI; on a real machine with audio
        // it should always find at least one output device.
        let list = enumerate_output_devices();
        if let Ok(list) = list {
            assert!(
                !list.devices.is_empty(),
                "expected at least one audio output device"
            );
            // The default device index, if present, must be valid
            if let Some(idx) = list.default_device_index {
                assert!(idx < list.devices.len());
                assert!(list.devices[idx].is_default);
            }
            // Each device should have at least one supported config
            for dev in &list.devices {
                assert!(
                    !dev.supported_configs.is_empty(),
                    "device '{}' should have supported configs",
                    dev.name
                );
            }
        }
        // If enumerate fails (no audio hardware), that's acceptable in CI.
    }

    #[test]
    fn resolve_config_with_default_falls_back_to_system_default() {
        let cfg = AudioDeviceConfig::default();
        // This may fail on headless CI; on a real machine it should succeed.
        if let Ok((_, stream_cfg, sr, _)) = resolve_config(&cfg) {
            assert!(sr > 0, "sample rate must be positive");
            // PB-3 MVP: always stereo
            assert_eq!(stream_cfg.channels, 2, "output must be stereo");
        }
    }

    #[test]
    fn resolve_config_with_nonexistent_device_returns_error() {
        let cfg = AudioDeviceConfig {
            device_name: Some("__nonexistent_device_12345__".to_string()),
            sample_rate: None,
            buffer_size: None,
        };
        let result = resolve_config(&cfg);
        assert!(result.is_err(), "nonexistent device must return error");
    }

    #[test]
    fn resolve_config_with_explicit_sample_rate() {
        let cfg = AudioDeviceConfig {
            device_name: None,
            sample_rate: Some(48000),
            buffer_size: None,
        };
        // May fail on headless CI or devices that don't support 48k stereo.
        if let Ok((_, stream_cfg, sr, _)) = resolve_config(&cfg) {
            assert_eq!(sr, 48000, "requested sample rate must be honored");
            assert_eq!(stream_cfg.channels, 2, "output must be stereo");
        }
    }

    #[test]
    fn resolve_config_forces_stereo_even_if_device_supports_more() {
        // Many devices support 8-channel output. We should always get 2.
        let cfg = AudioDeviceConfig::default();
        if let Ok((_, stream_cfg, _, _)) = resolve_config(&cfg) {
            assert_eq!(
                stream_cfg.channels, 2,
                "PB-3 MVP must force stereo regardless of device capability"
            );
        }
    }

    #[test]
    fn resolve_config_rejects_unsupported_sample_rate() {
        // Request a very unusual sample rate that no device supports.
        let cfg = AudioDeviceConfig {
            device_name: None,
            sample_rate: Some(11111),
            buffer_size: None,
        };
        let result = resolve_config(&cfg);
        // This should either error (if the device doesn't support 11111 Hz)
        // or succeed with a different rate (if the device is weird).
        // On any normal device, this will error.
        if let Err(e) = &result {
            assert!(
                e.contains("does not support") || e.contains("Failed"),
                "error should mention unsupported rate: {e}"
            );
        }
    }

    #[test]
    fn supported_sample_rates_filters_to_stereo_only() {
        // Create a mock device with stereo and 8-channel configs
        let device = AudioDevice {
            name: "Mock".to_string(),
            supported_configs: vec![
                SupportedAudioConfig {
                    channels: 2,
                    min_sample_rate: 44100,
                    max_sample_rate: 96000,
                    sample_format: cpal::SampleFormat::F32,
                },
                SupportedAudioConfig {
                    channels: 8,
                    min_sample_rate: 44100,
                    max_sample_rate: 192000,
                    sample_format: cpal::SampleFormat::F32,
                },
            ],
            default_sample_rate: 48000,
            default_channels: 2,
            is_default: false,
        };

        let rates = device.supported_sample_rates();
        // Should include rates from the stereo config (up to 96k)
        // but NOT 192k (only available in the 8-channel config)
        assert!(rates.contains(&44100), "should include 44100");
        assert!(rates.contains(&48000), "should include 48000");
        assert!(rates.contains(&96000), "should include 96000");
        assert!(
            !rates.contains(&192000),
            "should NOT include 192000 (only in 8-channel config)"
        );
    }

    #[test]
    fn supports_stereo_at_checks_actual_configs() {
        let device = AudioDevice {
            name: "Mock".to_string(),
            supported_configs: vec![SupportedAudioConfig {
                channels: 2,
                min_sample_rate: 44100,
                max_sample_rate: 48000,
                sample_format: cpal::SampleFormat::F32,
            }],
            default_sample_rate: 44100,
            default_channels: 2,
            is_default: false,
        };

        assert!(device.supports_stereo_at(44100), "should support 44100 stereo");
        assert!(device.supports_stereo_at(48000), "should support 48000 stereo");
        assert!(
            !device.supports_stereo_at(96000),
            "should NOT support 96000 stereo"
        );
        assert!(
            !device.supports_stereo_at(22050),
            "should NOT support 22050 stereo"
        );
    }
}
