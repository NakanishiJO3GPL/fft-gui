use rusb::UsbContext as _;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::task::AbortHandle;

// ── USB 設定 ─────────────────────────────────
// デバイスの VID / PID / エンドポイントに合わせて変更してください
const USB_VENDOR_ID: u16 = 0x1209;
const USB_PRODUCT_ID: u16 = 0x0001;
const USB_INTERFACE: u8 = 0;
const USB_BULK_IN_EP: u8 = 0x81; // Bulk IN エンドポイントアドレス
const USB_READ_TIMEOUT: Duration = Duration::from_millis(200);
const USB_SEQ_SIZE: usize = 2;                          // シーケンス番号のバイト数
const USB_EXPECTED_SIZE: usize = USB_SEQ_SIZE + FFT_BIN_COUNT; // 期待する最小サイズ (1026)
const USB_BULK_BUFFER_SIZE: usize = 4096;               // Overflow 防止のため余裕を持たせる

const FFT_BIN_COUNT: usize = 1024;
const FFT_EVENT_NAME: &str = "fft-data";

pub struct StreamState {
    abort_handle: Mutex<Option<AbortHandle>>,
    running: Arc<AtomicBool>,
    app: AppHandle,
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
    let running = Arc::clone(&state.running);
    running.store(true, Ordering::Relaxed);

    // rusb は同期 API なので spawn_blocking でラップする
    let join_handle = tokio::task::spawn_blocking(move || {
        // USB コンテキスト生成
        let context = match rusb::Context::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[fft] USB context error: {}", e);
                running.store(false, Ordering::Relaxed);
                return;
            }
        };

        // デバイスをオープン
        let handle: rusb::DeviceHandle<rusb::Context> = match context.open_device_with_vid_pid(USB_VENDOR_ID, USB_PRODUCT_ID) {
            Some(h) => h,
            None => {
                eprintln!(
                    "[fft] USB device {:04x}:{:04x} not found",
                    USB_VENDOR_ID, USB_PRODUCT_ID
                );
                running.store(false, Ordering::Relaxed);
                return;
            }
        };

        // インターフェースをクレーム
        if let Err(e) = handle.claim_interface(USB_INTERFACE) {
            eprintln!("[fft] claim_interface({}) failed: {}", USB_INTERFACE, e);
            running.store(false, Ordering::Relaxed);
            return;
        }

        eprintln!(
            "[fft] USB {:04x}:{:04x} opened, bulk stream started",
            USB_VENDOR_ID, USB_PRODUCT_ID
        );

        // Seq(2byte) + FFTデータ(1024byte) = 1026byte を1回で受け取る
        // バッファは余裕を持たせて Overflow を防ぐ
        let mut buf = vec![0u8; USB_BULK_BUFFER_SIZE];

        while running.load(Ordering::Relaxed) {
            match handle.read_bulk(USB_BULK_IN_EP, &mut buf, USB_READ_TIMEOUT) {
                Ok(n) if n >= USB_EXPECTED_SIZE => {
                    let fft_data = buf[USB_SEQ_SIZE..USB_EXPECTED_SIZE].to_vec();
                    if let Err(e) = app.emit(FFT_EVENT_NAME, fft_data) {
                        eprintln!("[fft] emit error: {}", e);
                    }
                }
                Ok(0) => {
                    // Zero Length Packet (ZLP) は転送終端の通知として正常。無視する
                }
                Ok(n) => {
                    eprintln!(
                        "[fft] unexpected packet size: {} bytes (expected >= {})",
                        n, USB_EXPECTED_SIZE
                    );
                }
                Err(rusb::Error::Timeout) => {
                    // 短いタイムアウトで running フラグを確認するための正常ケース
                }
                Err(e) => {
                    eprintln!("[fft] USB read error: {}", e);
                    break;
                }
            }
        }

        eprintln!("[fft] bulk stream stopped");
        running.store(false, Ordering::Relaxed);
    });

    *guard = Some(join_handle.abort_handle());

    Ok(())
}

#[tauri::command]
async fn stop_fft_stream(state: State<'_, StreamState>) -> Result<(), String> {
    // running フラグを先に落とす（ブロッキングループを抜けさせる）
    state.running.store(false, Ordering::Relaxed);

    let mut guard = state
        .abort_handle
        .lock()
        .map_err(|e| e.to_string())?;

    if let Some(handle) = guard.take() {
        handle.abort();
        eprintln!("[fft] stop requested");
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
                running: Arc::new(AtomicBool::new(false)),
                app: app.handle().clone(),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.try_state::<StreamState>() {
                    state.running.store(false, Ordering::Relaxed);
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
