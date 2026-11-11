use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub channels: Vec<u16>,
    pub sample_rates: Vec<u32>,
}

pub fn list_output_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    let host = cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|device| device.name().ok());

    let devices = host
        .output_devices()
        .map_err(|err| format!("failed to enumerate output devices: {err}"))?;

    let mut result = Vec::new();
    for (idx, device) in devices.enumerate() {
        let name = device
            .name()
            .unwrap_or_else(|_| format!("Output device {}", idx + 1));

        let mut channels = Vec::new();
        let mut sample_rates = Vec::new();
        if let Ok(configs) = device.supported_output_configs() {
            for config in configs {
                channels.push(config.channels());
                sample_rates.push(config.min_sample_rate().0);
                sample_rates.push(config.max_sample_rate().0);
            }
        }
        channels.sort_unstable();
        channels.dedup();
        sample_rates.sort_unstable();
        sample_rates.dedup();

        result.push(AudioDeviceInfo {
            id: format!("output-{idx}"),
            is_default: default_name.as_deref() == Some(name.as_str()),
            name,
            channels,
            sample_rates,
        });
    }

    Ok(result)
}
