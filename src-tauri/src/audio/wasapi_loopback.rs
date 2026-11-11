use std::{
    ffi::c_void,
    mem::size_of,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

use windows::core::implement;
use windows::Win32::{
    Foundation::{HANDLE, RPC_E_CHANGED_MODE},
    Media::{
        Audio::{
            eConsole, eRender, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
            IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
            IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT,
            AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS,
            AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
        },
        KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE},
    },
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
        CLSCTX_ALL, COINIT_MULTITHREADED,
    },
    UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    },
};
use windows_core::{Error as WinError, IUnknown, Interface, Ref, GUID, HRESULT};

use crate::audio::sources::{ApplicationCaptureTarget, CaptureSource};

const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
    GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const SILENCE_KEEPALIVE_INTERVAL: Duration = Duration::from_millis(20);
const SILENCE_KEEPALIVE_HZ: u32 = 50;
const PROCESS_LOOPBACK_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(8);

pub struct AudioChunk {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct AudioActivationHandler {
    event: HANDLE,
}

#[allow(non_snake_case)]
impl IActivateAudioInterfaceCompletionHandler_Impl for AudioActivationHandler_Impl {
    fn ActivateCompleted(
        &self,
        _activateoperation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows_core::Result<()> {
        unsafe {
            windows::Win32::System::Threading::SetEvent(self.event).ok();
        }
        Ok(())
    }
}

struct ComGuard {
    uninitialize_on_drop: bool,
}

impl ComGuard {
    fn init() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(Self {
                uninitialize_on_drop: false,
            });
        }
        result
            .ok()
            .map_err(|err| format!("failed to initialize COM: {err}"))?;
        Ok(Self {
            uninitialize_on_drop: true,
        })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninitialize_on_drop {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

pub fn capture_loop<F>(stop: Arc<AtomicBool>, mut on_chunk: F) -> Result<(), String>
where
    F: FnMut(AudioChunk) + Send + 'static,
{
    let _com = ComGuard::init()?;

    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|err| format!("failed to create MMDeviceEnumerator: {err}"))?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|err| format!("failed to get default render endpoint: {err}"))?;
        let audio_client: IAudioClient = device
            .Activate::<IAudioClient>(CLSCTX_ALL, None)
            .map_err(|err| format!("failed to activate IAudioClient: {err}"))?;

        capture_from_audio_client_with_flags(audio_client, stop, AUDCLNT_STREAMFLAGS_LOOPBACK, &mut on_chunk)
    }
}

pub fn capture_loop_for_source<F>(
    stop: Arc<AtomicBool>,
    source: CaptureSource,
    on_chunk: F,
) -> Result<(), String>
where
    F: FnMut(AudioChunk) + Send + 'static,
{
    match source {
        CaptureSource::System => capture_loop(stop, on_chunk),
        CaptureSource::Applications { apps } => capture_loop_for_applications(stop, apps, on_chunk),
    }
}

fn capture_loop_for_process<F>(
    stop: Arc<AtomicBool>,
    process_id: u32,
    mut on_chunk: F,
) -> Result<(), String>
where
    F: FnMut(AudioChunk) + Send + 'static,
{
    let _com = ComGuard::init()?;
    let audio_client = activate_process_loopback_client(process_id)?;
    capture_from_audio_client_with_flags(audio_client, stop, 0, &mut on_chunk)
}

fn capture_loop_for_applications<F>(
    stop: Arc<AtomicBool>,
    apps: Vec<ApplicationCaptureTarget>,
    mut on_chunk: F,
) -> Result<(), String>
where
    F: FnMut(AudioChunk) + Send + 'static,
{
    let apps = apps
        .into_iter()
        .filter(|app| app.process_id > 0)
        .collect::<Vec<_>>();
    if apps.is_empty() {
        return Err("no application capture targets selected".to_string());
    }
    if apps.len() == 1 {
        return capture_loop_for_process(stop, apps[0].process_id, on_chunk);
    }

    let (tx, rx) = mpsc::channel::<CaptureMessage>();
    let mut workers = Vec::new();
    for app in apps {
        let chunk_tx = tx.clone();
        let error_tx = tx.clone();
        let worker_stop = Arc::clone(&stop);
        workers.push(thread::spawn(move || {
            let result = capture_loop_for_process(worker_stop, app.process_id, move |chunk| {
                let _ = chunk_tx.send(CaptureMessage::Chunk(chunk));
            });
            if let Err(err) = result {
                let _ = error_tx.send(CaptureMessage::Error(format!("{}: {err}", app.name)));
            }
        }));
    }
    drop(tx);

    let mut last_sample_rate = 48_000;
    let mut last_channels = 2;
    let mut last_error = None::<String>;

    while !stop.load(Ordering::Relaxed) {
        let frame_started = Instant::now();
        let mut chunks = Vec::<AudioChunk>::new();

        loop {
            let remaining = SILENCE_KEEPALIVE_INTERVAL.saturating_sub(frame_started.elapsed());
            if remaining.is_zero() {
                break;
            }

            match rx.recv_timeout(remaining) {
                Ok(CaptureMessage::Chunk(chunk)) => {
                    last_sample_rate = chunk.sample_rate;
                    last_channels = chunk.channels;
                    chunks.push(chunk);
                }
                Ok(CaptureMessage::Error(err)) => {
                    last_error = Some(err);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    stop.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }

        if chunks.is_empty() {
            on_chunk(keepalive_silence_chunk(last_sample_rate, last_channels));
        } else {
            on_chunk(mix_chunks(chunks));
        }
    }

    for worker in workers {
        let _ = worker.join();
    }

    if let Some(err) = last_error {
        Err(err)
    } else {
        Ok(())
    }
}

enum CaptureMessage {
    Chunk(AudioChunk),
    Error(String),
}

fn activate_process_loopback_client(process_id: u32) -> Result<IAudioClient, String> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: process_id,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };

    let mut activation_params: windows::Win32::System::Com::StructuredStorage::PROPVARIANT =
        unsafe { std::mem::zeroed() };

    unsafe {
        let inner = &mut *activation_params.Anonymous.Anonymous;
        inner.vt = windows::Win32::System::Variant::VARENUM(0x1007u16);
        inner.Anonymous.blob.cbSize = size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32;
        inner.Anonymous.blob.pBlobData = &params as *const _ as *mut u8;
    }

    let event = unsafe {
        windows::Win32::System::Threading::CreateEventW(None, true, false, None)
            .map_err(|err| format!("failed to create activation event: {err}"))?
    };

    let handler: IActivateAudioInterfaceCompletionHandler = AudioActivationHandler { event }.into();

    let activation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&activation_params as *const _),
            &handler,
        )
    };
    let operation = match activation {
        Ok(operation) => operation,
        Err(err) => {
            return Err(format!("process loopback activation failed: {err}"));
        }
    };

    let result = wait_for_activation_completed(event, PROCESS_LOOPBACK_ACTIVATION_TIMEOUT)
        .and_then(|()| read_activation_result(&operation));
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(event) };
    result
}

fn wait_for_activation_completed(event: HANDLE, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut msg = MSG::default();
    let pump_interval = Duration::from_millis(5);

    loop {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let wait_ms = pump_interval.as_millis().min(50) as u32;
        let wait_result = unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(event, wait_ms)
        };

        match wait_result.0 {
            0 => return Ok(()),
            258 => {
                if Instant::now() >= deadline {
                    return Err("process loopback activation timed out".to_string());
                }
            }
            other => {
                return Err(format!(
                    "process loopback activation failed with wait result: {other}"
                ));
            }
        }
    }
}

fn read_activation_result(
    operation: &IActivateAudioInterfaceAsyncOperation,
) -> Result<IAudioClient, String> {
    let mut activate_result = HRESULT(0);
    let mut activated_interface: Option<IUnknown> = None;
    unsafe {
        operation
            .GetActivateResult(&mut activate_result, &mut activated_interface)
            .map_err(|err| format!("failed to read process loopback activation result: {err}"))?;
    }
    activate_result.ok().map_err(|_| {
        format!(
            "process loopback activation failed: {}",
            WinError::from_hresult(activate_result)
        )
    })?;
    activated_interface
        .ok_or_else(|| "process loopback activation returned no audio client".to_string())?
        .cast::<IAudioClient>()
        .map_err(|err| format!("process loopback activation did not return IAudioClient: {err}"))
}

fn capture_from_audio_client_with_flags<F>(
    audio_client: IAudioClient,
    stop: Arc<AtomicBool>,
    stream_flags: u32,
    on_chunk: &mut F,
) -> Result<(), String>
where
    F: FnMut(AudioChunk),
{
    unsafe {
        let mix_format_ptr = audio_client
            .GetMixFormat()
            .map_err(|err| format!("failed to get mix format: {err}"))?;
        if mix_format_ptr.is_null() {
            return Err("WASAPI returned a null mix format".to_string());
        }

        let mix_format = *mix_format_ptr;
        let sample_rate = mix_format.nSamplesPerSec;
        let channels = mix_format.nChannels;
        let sample_format = detect_sample_format(mix_format_ptr)?;

        let buffer_duration_100ns = 100_0000;
        audio_client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                stream_flags,
                buffer_duration_100ns,
                0,
                mix_format_ptr,
                None,
            )
            .map_err(|err| format!("failed to initialize WASAPI client: {err}"))?;

        CoTaskMemFree(Some(mix_format_ptr as *const c_void));

        let capture_client: IAudioCaptureClient = audio_client
            .GetService()
            .map_err(|err| format!("failed to get IAudioCaptureClient: {err}"))?;
        audio_client
            .Start()
            .map_err(|err| format!("failed to start WASAPI capture: {err}"))?;

        let result = run_capture_loop(
            &capture_client,
            stop,
            sample_rate,
            channels,
            sample_format,
            on_chunk,
        );

        let _ = audio_client.Stop();
        result
    }
}

#[derive(Debug, Clone, Copy)]
enum SampleFormat {
    F32,
    I16,
}

unsafe fn detect_sample_format(format: *mut WAVEFORMATEX) -> Result<SampleFormat, String> {
    let fmt = *format;
    if fmt.wFormatTag == WAVE_FORMAT_EXTENSIBLE as u16 {
        let extensible = *(format as *const WAVEFORMATEXTENSIBLE);
        let subtype: GUID = extensible.SubFormat;
        if subtype == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            return Ok(SampleFormat::F32);
        }
        if subtype == KSDATAFORMAT_SUBTYPE_PCM {
            return Ok(SampleFormat::I16);
        }
    }

    let format_tag = fmt.wFormatTag;
    let bits_per_sample = fmt.wBitsPerSample;

    match (format_tag, bits_per_sample) {
        (3, 32) => Ok(SampleFormat::F32),
        (1, 16) => Ok(SampleFormat::I16),
        _ => Err(format!(
            "unsupported WASAPI mix format: tag={}, bits={}",
            format_tag, bits_per_sample
        )),
    }
}

fn run_capture_loop<F>(
    capture_client: &IAudioCaptureClient,
    stop: Arc<AtomicBool>,
    sample_rate: u32,
    channels: u16,
    sample_format: SampleFormat,
    on_chunk: &mut F,
) -> Result<(), String>
where
    F: FnMut(AudioChunk),
{
    let mut last_chunk_at = Instant::now() - SILENCE_KEEPALIVE_INTERVAL;
    while !stop.load(Ordering::Relaxed) {
        let mut packet_frames = unsafe {
            capture_client
                .GetNextPacketSize()
                .map_err(|err| format!("failed to get next WASAPI packet size: {err}"))?
        };

        if packet_frames == 0 {
            if last_chunk_at.elapsed() >= SILENCE_KEEPALIVE_INTERVAL {
                on_chunk(keepalive_silence_chunk(sample_rate, channels));
                last_chunk_at = Instant::now();
            }
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        while packet_frames > 0 {
            let mut data: *mut u8 = null_mut();
            let mut frame_count = 0u32;
            let mut flags = 0u32;

            unsafe {
                capture_client
                    .GetBuffer(&mut data, &mut frame_count, &mut flags, None, None)
                    .map_err(|err| format!("failed to read WASAPI capture buffer: {err}"))?;
            }

            let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                vec![0.0; frame_count as usize * channels as usize]
            } else {
                convert_samples(data, frame_count, channels, sample_format)
            };

            unsafe {
                capture_client
                    .ReleaseBuffer(frame_count)
                    .map_err(|err| format!("failed to release WASAPI capture buffer: {err}"))?;
                packet_frames = capture_client
                    .GetNextPacketSize()
                    .map_err(|err| format!("failed to get next WASAPI packet size: {err}"))?;
            }

            on_chunk(AudioChunk {
                sample_rate,
                channels,
                samples,
            });
            last_chunk_at = Instant::now();
        }
    }

    Ok(())
}

fn keepalive_silence_chunk(sample_rate: u32, channels: u16) -> AudioChunk {
    let channels = channels.max(1);
    let frames = (sample_rate / SILENCE_KEEPALIVE_HZ).max(1);
    AudioChunk {
        sample_rate,
        channels,
        samples: vec![0.0; frames as usize * channels as usize],
    }
}

fn mix_chunks(chunks: Vec<AudioChunk>) -> AudioChunk {
    let sample_rate = chunks
        .first()
        .map(|chunk| chunk.sample_rate)
        .unwrap_or(48_000);
    let channels = chunks
        .first()
        .map(|chunk| chunk.channels)
        .unwrap_or(2)
        .max(1);
    let max_len = chunks
        .iter()
        .map(|chunk| chunk.samples.len())
        .max()
        .unwrap_or_default();
    let mut samples = vec![0.0_f32; max_len];
    for chunk in &chunks {
        for (index, sample) in chunk.samples.iter().enumerate().take(max_len) {
            samples[index] = (samples[index] + *sample).clamp(-1.0, 1.0);
        }
    }

    AudioChunk {
        sample_rate,
        channels,
        samples,
    }
}

fn convert_samples(
    data: *const u8,
    frame_count: u32,
    channels: u16,
    sample_format: SampleFormat,
) -> Vec<f32> {
    if data.is_null() {
        return vec![0.0; frame_count as usize * channels as usize];
    }

    match sample_format {
        SampleFormat::F32 => {
            let sample_count = frame_count as usize * channels as usize;
            let input = unsafe { std::slice::from_raw_parts(data as *const f32, sample_count) };
            input.to_vec()
        }
        SampleFormat::I16 => {
            let sample_count = frame_count as usize * channels as usize;
            let input = unsafe { std::slice::from_raw_parts(data as *const i16, sample_count) };
            input
                .iter()
                .map(|sample| *sample as f32 / i16::MAX as f32)
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_silence_chunk_is_twenty_milliseconds() {
        let chunk = keepalive_silence_chunk(48_000, 2);
        assert_eq!(chunk.sample_rate, 48_000);
        assert_eq!(chunk.channels, 2);
        assert_eq!(chunk.samples.len(), 1_920);
        assert!(chunk.samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn keepalive_silence_chunk_clamps_empty_channel_count() {
        let chunk = keepalive_silence_chunk(44_100, 0);
        assert_eq!(chunk.channels, 1);
        assert_eq!(chunk.samples.len(), 882);
    }

    #[test]
    fn mix_chunks_sums_and_clamps_samples() {
        let mixed = mix_chunks(vec![
            AudioChunk {
                sample_rate: 48_000,
                channels: 2,
                samples: vec![0.6, -0.6, 0.1],
            },
            AudioChunk {
                sample_rate: 48_000,
                channels: 2,
                samples: vec![0.7, -0.7],
            },
        ]);
        assert_eq!(mixed.sample_rate, 48_000);
        assert_eq!(mixed.channels, 2);
        assert_eq!(mixed.samples, vec![1.0, -1.0, 0.1]);
    }
}
