use std::f32::consts::PI;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

const FFT_BIN_COUNT: usize = 1024;
const FFT_EVENT_NAME: &str = "fft-data";
const FFT_FRAME_INTERVAL_MS: u64 = 50;

pub struct StreamState {
    running: Arc<AtomicBool>,
    app: AppHandle,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn clamp_to_u8(v: f32) -> u8 {
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v.round() as u8
    }
}

fn generate_sample_fft(frame: u64) -> Vec<u8> {
    let mut out = vec![0u8; FFT_BIN_COUNT];
    let t = frame as f32;

    for i in 0..FFT_BIN_COUNT {
        let x = i as f32;

        let envelope = 180.0 * (-x / 480.0).exp();

        let p1_center = 120.0 + (t * 0.07).sin() * 28.0;
        let p2_center = 360.0 + (t * 0.05).sin() * 54.0;
        let p3_center = 740.0 + (t * 0.06).cos() * 42.0;

        let peak1 = 85.0 * (-((x - p1_center) / 18.0).powi(2)).exp();
        let peak2 = 65.0 * (-((x - p2_center) / 28.0).powi(2)).exp();
        let peak3 = 48.0 * (-((x - p3_center) / 35.0).powi(2)).exp();

        let ripple = 10.0 * (1.0 + ((x * 0.08) + (t * 0.22) * PI / 2.0).sin());
        let floor = 3.0 + 2.0 * ((x * 0.013) + t * 0.04).cos();

        let y = envelope * 0.2 + peak1 + peak2 + peak3 + ripple + floor;
        out[i] = clamp_to_u8(y);
    }

    out
}

#[tauri::command]
fn start_fft_stream(state: State<'_, StreamState>) {
    let already = state.running.swap(true, Ordering::Relaxed);
    if already {
        eprintln!("[fft] stream already running, ignoring start request");
        return;
    }

    let app = state.app.clone();
    let running = Arc::clone(&state.running);

    eprintln!("[fft] stream started");

    thread::spawn(move || {
        let mut frame: u64 = 0;

        while running.load(Ordering::Relaxed) {
            let fft = generate_sample_fft(frame);

            if let Err(err) = app.emit(FFT_EVENT_NAME, fft) {
                eprintln!("[fft] failed to emit {}: {}", FFT_EVENT_NAME, err);
            }

            frame = frame.wrapping_add(1);
            thread::sleep(Duration::from_millis(FFT_FRAME_INTERVAL_MS));
        }

        eprintln!("[fft] stream stopped");
    });
}

#[tauri::command]
fn stop_fft_stream(state: State<'_, StreamState>) {
    state.running.store(false, Ordering::Relaxed);
    eprintln!("[fft] stop requested");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            app.manage(StreamState {
                running: Arc::new(AtomicBool::new(false)),
                app: app.handle().clone(),
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.try_state::<StreamState>() {
                    state.running.store(false, Ordering::Relaxed);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![greet, start_fft_stream, stop_fft_stream])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}