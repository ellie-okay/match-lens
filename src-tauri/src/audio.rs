use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::slice;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::info;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
    WAVE_FORMAT_PCM,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

pub struct LoopbackCaptureSession {
    output_path: PathBuf,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<PathBuf>>>,
}

impl LoopbackCaptureSession {
    pub fn start(output_path: &Path) -> Result<Self> {
        let output_path = output_path.to_path_buf();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let thread_stop_flag = Arc::clone(&stop_flag);
        let thread_path = output_path.clone();
        let (started_tx, started_rx) = mpsc::sync_channel::<Result<(), String>>(1);

        let thread = thread::Builder::new()
            .name("loopback-audio-capture".into())
            .spawn(move || capture_loopback_to_wav(thread_stop_flag, thread_path, started_tx))
            .context("Failed to spawn loopback audio capture thread")?;

        match started_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow!(e)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(anyhow!(
                    "Timed out waiting for loopback audio capture startup"
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow!(
                    "Loopback audio capture thread exited during startup"
                ));
            }
        }

        Ok(Self {
            output_path,
            stop_flag,
            thread: Some(thread),
        })
    }

    pub fn stop(mut self) -> Result<PathBuf> {
        self.stop_flag.store(true, Ordering::Relaxed);

        let handle = self
            .thread
            .take()
            .ok_or_else(|| anyhow!("Loopback audio capture thread missing"))?;

        handle
            .join()
            .map_err(|_| anyhow!("Loopback audio capture thread panicked"))?
    }

    pub fn output_path(&self) -> &Path {
        &self.output_path
    }
}

fn capture_loopback_to_wav(
    stop_flag: Arc<AtomicBool>,
    output_path: PathBuf,
    started_tx: mpsc::SyncSender<Result<(), String>>,
) -> Result<PathBuf> {
    let capture = match initialize_capture(&output_path) {
        Ok(capture) => {
            let _ = started_tx.send(Ok(()));
            capture
        }
        Err(e) => {
            let _ = started_tx.send(Err(e.to_string()));
            return Err(e);
        }
    };

    let CaptureState {
        _com_guard,
        audio_client,
        capture_client,
        wave_format,
        mut writer,
    } = capture;

    while !stop_flag.load(Ordering::Relaxed) {
        if !drain_capture_packets(
            &capture_client,
            &mut writer,
            wave_format.nBlockAlign as usize,
        )? {
            thread::sleep(Duration::from_millis(5));
        }
    }

    let _ = drain_capture_packets(
        &capture_client,
        &mut writer,
        wave_format.nBlockAlign as usize,
    );

    unsafe { audio_client.Stop() }.context("Failed to stop loopback audio capture")?;
    writer.finish()?;

    info!("Loopback audio capture finalized: {:?}", output_path);
    Ok(output_path)
}

fn initialize_capture(output_path: &Path) -> Result<CaptureState> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
        .context("Failed to initialize COM for loopback audio capture")?;
    let com_guard = ComGuard;

    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .context("Failed to create audio device enumerator")?;

    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
        .context("Failed to get the default render audio endpoint")?;

    let audio_client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
        .context("Failed to activate the default render endpoint")?;

    let wave_format = pcm_stereo_48khz_wave_format();
    let stream_flags = AUDCLNT_STREAMFLAGS_LOOPBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

    unsafe {
        audio_client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            stream_flags,
            0,
            0,
            &wave_format,
            None,
        )
    }
    .context("Failed to initialize the loopback audio client")?;

    let capture_client: IAudioCaptureClient =
        unsafe { audio_client.GetService() }.context("Failed to get audio capture client")?;

    let writer = WavWriter::create(output_path, &wave_format)?;

    unsafe { audio_client.Start() }.context("Failed to start loopback audio capture")?;
    info!("Loopback audio capture started: {:?}", output_path);

    Ok(CaptureState {
        _com_guard: com_guard,
        audio_client,
        capture_client,
        wave_format,
        writer,
    })
}

fn drain_capture_packets(
    capture_client: &IAudioCaptureClient,
    writer: &mut WavWriter,
    block_align: usize,
) -> Result<bool> {
    let mut wrote_any = false;

    loop {
        let packet_frames =
            unsafe { capture_client.GetNextPacketSize() }.context("Failed to query packet size")?;

        if packet_frames == 0 {
            return Ok(wrote_any);
        }

        let mut data = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;

        unsafe { capture_client.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }
            .context("Failed to read loopback audio buffer")?;

        let byte_count = frames as usize * block_align;
        if (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
            writer.write_frames(&vec![0u8; byte_count])?;
        } else if byte_count > 0 {
            let bytes = unsafe { slice::from_raw_parts(data as *const u8, byte_count) };
            writer.write_frames(bytes)?;
        }

        unsafe { capture_client.ReleaseBuffer(frames) }
            .context("Failed to release loopback audio buffer")?;

        wrote_any = true;
    }
}

fn pcm_stereo_48khz_wave_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * 2 * 2,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    }
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct CaptureState {
    _com_guard: ComGuard,
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    wave_format: WAVEFORMATEX,
    writer: WavWriter,
}

struct WavWriter {
    file: File,
    data_bytes: u32,
}

impl WavWriter {
    fn create(path: &Path, wave_format: &WAVEFORMATEX) -> Result<Self> {
        let mut file = File::create(path)
            .with_context(|| format!("Failed to create WAV file at {}", path.display()))?;

        file.write_all(b"RIFF")?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(b"WAVE")?;
        file.write_all(b"fmt ")?;
        file.write_all(&16u32.to_le_bytes())?;
        file.write_all(&wave_format.wFormatTag.to_le_bytes())?;
        file.write_all(&wave_format.nChannels.to_le_bytes())?;
        file.write_all(&wave_format.nSamplesPerSec.to_le_bytes())?;
        file.write_all(&wave_format.nAvgBytesPerSec.to_le_bytes())?;
        file.write_all(&wave_format.nBlockAlign.to_le_bytes())?;
        file.write_all(&wave_format.wBitsPerSample.to_le_bytes())?;
        file.write_all(b"data")?;
        file.write_all(&0u32.to_le_bytes())?;

        Ok(Self {
            file,
            data_bytes: 0,
        })
    }

    fn write_frames(&mut self, bytes: &[u8]) -> Result<()> {
        self.file.write_all(bytes)?;
        self.data_bytes = self
            .data_bytes
            .checked_add(bytes.len() as u32)
            .ok_or_else(|| anyhow!("WAV file exceeded 4 GB"))?;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.file.flush()?;
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&(36 + self.data_bytes).to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&self.data_bytes.to_le_bytes())?;
        self.file.flush()?;
        Ok(())
    }
}
