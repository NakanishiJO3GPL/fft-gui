use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::task::AbortHandle;
use hidapi::HidApi;

const FFT_BIN_COUNT: usize = 1024;
const FFT_EVENT_NAME: &str = "fft-data";
const VENDOR_ID: u16 = 0x1209;
const PRODUCT_ID: u16 = 0x0001;
const REPORT_SIZE: usize = 256;
const REPORT_HEADER_SIZE: usize = 4;
const REPORT_DATA_SIZE: usize = REPORT_SIZE - REPORT_HEADER_SIZE;

pub struct StreamState {
    abort_handle: Mutex<Option<AbortHandle>>,
    app: AppHandle,
}

#[derive(Debug, Clone)]
struct HidPacket {
    pub seq: u16,
    pub offset: u16,
    pub data: [u8; REPORT_DATA_SIZE],
}
impl HidPacket {
    pub fn new() -> Self {
        Self {
            seq: 0,
            offset: 0,
            data: [0u8; REPORT_DATA_SIZE],
        }
    }
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
async fn start_fft_stream(state: State<'_, StreamState>) -> Result<(), String> {
    let mut guard = state
        .abort_handle
        .lock()
        .map_err(|e| e.to_string())?;

    if guard.is_some() {
        eprintln!("[fft] stream already running, ignoring start request");
        return Ok(());
    }

    let app = state.app.clone();

    let join_handle = tokio::task::spawn(async move {
        eprintln!("[fft] stream started");

        // Channel between HID reader and main loop
        let (tx, mut rx) = tokio::sync::mpsc::channel::<HidPacket>(2);

        // Start HID device
        let api = HidApi::new().map_err(|e| {
            eprintln!("[fft] failed to open HID device: {}", e);
            return;
        }).unwrap();

        let device = api.open(VENDOR_ID, PRODUCT_ID).map_err(|e| {
            eprintln!("[fft] failed to open HID device: {}", e);
            return;
        }).unwrap();

        tokio::task::spawn(async move {
            let mut report = [0u8; REPORT_SIZE];
            let mut packet = HidPacket::new();
            loop {
                let n = device.read_timeout(&mut report, 100).map_err(|e| {
                    eprintln!("[fft] failed to read HID device: {}", e);
                }).unwrap();
                if n == 0 || n != REPORT_SIZE {
                    continue;
                }
                packet.seq = u16::from_le_bytes([report[0], report[1]]);
                packet.offset = u16::from_le_bytes([report[2], report[3]]);
                packet.data = report[4..].try_into().unwrap();

                if let Err(e) = tx.send(packet.clone()).await {
                    eprintln!("[fft] failed to send packet: {}", e);
                }
            }
        });

        // Start main loop
        let mut spectrum = [0u8; FFT_BIN_COUNT];
        loop {
            if let Some(packet) = rx.recv().await {
                let offset_end = (packet.offset as usize + packet.data.len()).min(1023);
                let source_end = packet.data.len().min(offset_end - packet.offset as usize);
                println!("offset_end = {}, source_end = {}", offset_end, source_end);
                spectrum[packet.offset as usize..offset_end]
                    .copy_from_slice(&packet.data[..source_end]);
            }

            if let Err(err) = app.emit(FFT_EVENT_NAME, spectrum.to_vec()) {
                eprintln!("[fft] failed to emit {}: {}", FFT_EVENT_NAME, err);
            }
        }
    });

    *guard = Some(join_handle.abort_handle());

    Ok(())
}

#[tauri::command]
async fn stop_fft_stream(state: State<'_, StreamState>) -> Result<(), String> {
    let mut guard = state
        .abort_handle
        .lock()
        .map_err(|e| e.to_string())?;

    if let Some(handle) = guard.take() {
        handle.abort();
        eprintln!("[fft] stream stopped");
    } else {
        eprintln!("[fft] stop requested but stream was not running");
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            app.manage(StreamState {
                abort_handle: Mutex::new(None),
                app: app.handle().clone(),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.try_state::<StreamState>() {
                    if let Ok(mut guard) = state.abort_handle.lock() {
                        if let Some(handle) = guard.take() {
                            handle.abort();
                            eprintln!("[fft] stream aborted on window close");
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            start_fft_stream,
            stop_fft_stream
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
