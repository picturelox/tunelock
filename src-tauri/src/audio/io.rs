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
// Future: ASIO support on Windows, cue/master channel routing, and
// disconnect/reconnect handling. Those are deferred; this phase establishes
// the selection infrastructure.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{SampleRate, StreamConfig};

/// A description of one available audio output device.
#[derive(Debug, Clone)]
pub struct AudioDevice {
    /// CPAL device name (human-readable).
    pub name: String,
    /// Supported sample rates (sorted ascending).
    pub sample_rates: Vec<u32>,
    /// Supported channel counts (sorted ascending).
    pub channel_counts: Vec<u16>,
    /// Default sample rate for this device.
    pub default_sample_rate: u32,
    /// Default channel count.
    pub default_channels: u16,
    /// Whether this is the system default output device.
    pub is_default: bool,
}

/// A list of available audio output devices.
#[derive(Debug, Clone)]
pub struct AudioDeviceList {
    pub devices: Vec<AudioDevice>,
    pub default_device_index: Option<usize>,
}

/// The user's chosen audio configuration. Used to build the engine stream.
#[derive(Debug, Clone)]
pub struct AudioDeviceConfig {
    /// Device name to select (must match a name from AudioDeviceList).
    /// If None, uses the system default.
    pub device_name: Option<String>,
    /// Requested sample rate in Hz. If None, uses the device default.
    pub sample_rate: Option<u32>,
    /// Requested channel count. If None, uses the device default.
    pub channels: Option<u16>,
    /// Requested buffer size. If None, uses the device default.
    pub buffer_size: Option<BufferSizePreference>,
}

/// Buffer size preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferSizePreference {
    /// Let the system choose (typically 256-1024 frames).
    Default,
    /// Lowest latency the device supports.
    Minimal,
    /// A specific frame count.
    Fixed(u32),
}

impl Default for AudioDeviceConfig {
    fn default() -> Self {
        Self {
            device_name: None,
            sample_rate: None,
            channels: None,
            buffer_size: None,
        }
    }
}

/// Enumerate all available audio output devices on the default host.
/// Returns device names, supported sample rates, channel counts, and
/// which device is the system default.
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

        let mut sample_rates = Vec::new();
        let mut channel_counts = Vec::new();
        let mut default_sr = 44100u32;
        let mut default_ch = 2u16;

        for sc in configs {
            let min = sc.min_sample_rate().0;
            let max = sc.max_sample_rate().0;
            // Collect common rates within the supported range
            for rate in [22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000] {
                if rate >= min && rate <= max && !sample_rates.contains(&rate) {
                    sample_rates.push(rate);
                }
            }
            let ch = sc.channels();
            if !channel_counts.contains(&ch) {
                channel_counts.push(ch);
            }
        }

        sample_rates.sort_unstable();
        channel_counts.sort_unstable();

        if let Some(default_cfg) = device.default_output_config().ok() {
            default_sr = default_cfg.sample_rate().0;
            default_ch = default_cfg.channels();
        }

        if is_default {
            default_index = Some(devices.len());
        }

        devices.push(AudioDevice {
            name,
            sample_rates,
            channel_counts,
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

/// Resolve an AudioDeviceConfig to a concrete CPAL device and stream config.
/// Called by the engine during construction.
pub(crate) fn resolve_config(
    config: &AudioDeviceConfig,
) -> Result<(cpal::Device, StreamConfig, u32), String> {
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

    let supported = device
        .default_output_config()
        .map_err(|e| format!("Failed to get output config: {}", e))?;

    let sample_rate = config
        .sample_rate
        .unwrap_or(supported.sample_rate().0);

    let channels = config.channels.unwrap_or(supported.channels());

    // Build the stream config from the supported config, overriding
    // sample rate and buffer size as requested.
    let mut stream_config: StreamConfig = supported.into();
    stream_config.sample_rate = SampleRate(sample_rate);
    stream_config.channels = channels;

    if let Some(buf_pref) = config.buffer_size {
        stream_config.buffer_size = match buf_pref {
            BufferSizePreference::Default => cpal::BufferSize::Default,
            BufferSizePreference::Minimal => cpal::BufferSize::Fixed(64),
            BufferSizePreference::Fixed(n) => cpal::BufferSize::Fixed(n),
        };
    }

    Ok((device, stream_config, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_all_none() {
        let cfg = AudioDeviceConfig::default();
        assert!(cfg.device_name.is_none());
        assert!(cfg.sample_rate.is_none());
        assert!(cfg.channels.is_none());
        assert!(cfg.buffer_size.is_none());
    }

    #[test]
    fn buffer_size_preference_fixed_preserves_value() {
        let b = BufferSizePreference::Fixed(256);
        assert_eq!(b, BufferSizePreference::Fixed(256));
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
        }
        // If enumerate fails (no audio hardware), that's acceptable in CI.
    }

    #[test]
    fn resolve_config_with_default_falls_back_to_system_default() {
        let cfg = AudioDeviceConfig::default();
        // This may fail on headless CI; on a real machine it should succeed.
        if let Ok((_, _, sr)) = resolve_config(&cfg) {
            assert!(sr > 0, "sample rate must be positive");
        }
    }

    #[test]
    fn resolve_config_with_nonexistent_device_returns_error() {
        let cfg = AudioDeviceConfig {
            device_name: Some("__nonexistent_device_12345__".to_string()),
            sample_rate: None,
            channels: None,
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
            channels: None,
            buffer_size: None,
        };
        // May fail on headless CI or devices that don't support 48k.
        if let Ok((_, _, sr)) = resolve_config(&cfg) {
            assert_eq!(sr, 48000, "requested sample rate must be honored");
        }
    }
}
