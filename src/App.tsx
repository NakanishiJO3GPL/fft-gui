import { useEffect, useMemo, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import {
  CategoryScale,
  Chart as ChartJS,
  Legend,
  LineElement,
  LinearScale,
  PointElement,
  Title,
  Tooltip,
  type ChartData,
  type ChartOptions,
} from "chart.js";
import { Line } from "react-chartjs-2";
import "./App.css";

ChartJS.register(
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Title,
  Tooltip,
  Legend,
);

const FFT_BIN_COUNT = 1024;
const FFT_MIN = 0;
const FFT_MAX = 255;
const FFT_EVENT_NAME = "fft-data";
const SAMPLE_RATE = 48000;
const HZ_PER_BIN = SAMPLE_RATE / 2 / FFT_BIN_COUNT; // 23.4375 Hz/bin (Nyquist = 24kHz)

function clampTo8bit(value: number): number {
  if (Number.isNaN(value)) return 0;
  return Math.max(FFT_MIN, Math.min(FFT_MAX, Math.round(value)));
}

function normalizeFftArray(input: number[]): number[] {
  const out = new Array<number>(FFT_BIN_COUNT).fill(0);
  const len = Math.min(input.length, FFT_BIN_COUNT);
  for (let i = 0; i < len; i++) {
    out[i] = clampTo8bit(input[i]);
  }
  return out;
}

function App() {
  const chartRef = useRef<ChartJS<"line"> | null>(null);

  // bin i → 周波数 [Hz]
  const labels = useMemo(
    () =>
      Array.from({ length: FFT_BIN_COUNT }, (_, i) => {
        const hz = i * HZ_PER_BIN;
        return hz.toFixed(0);
      }),
    [],
  );

  const initialData = useMemo<ChartData<"line">>(
    () => ({
      labels,
      datasets: [
        {
          label: "FFT Amplitude (8bit)",
          data: new Array<number>(FFT_BIN_COUNT).fill(0),
          borderColor: "#3b82f6",
          backgroundColor: "rgba(59, 130, 246, 0.15)",
          borderWidth: 1.5,
          pointRadius: 0,
          tension: 0,
          fill: true,
        },
      ],
    }),
    [labels],
  );

  const options = useMemo<ChartOptions<"line">>(
    () => ({
      responsive: true,
      maintainAspectRatio: false,
      animation: false,
      plugins: {
        legend: {
          display: true,
          position: "top",
        },
        title: {
          display: true,
          text: `1024-bin 8-bit FFT  (fs = ${(SAMPLE_RATE / 1000).toFixed(0)} kHz)`,
        },
        tooltip: {
          mode: "index",
          intersect: false,
          callbacks: {
            title: (items) => {
              const hz = parseFloat(items[0]?.label ?? "0");
              return hz >= 1000
                ? `${(hz / 1000).toFixed(2)} kHz`
                : `${hz.toFixed(0)} Hz`;
            },
            label: (item) => `amplitude: ${item.parsed.y}`,
          },
        },
      },
      interaction: {
        mode: "nearest",
        axis: "x",
        intersect: false,
      },
      scales: {
        x: {
          title: {
            display: true,
            text: "Frequency (Hz)",
          },
          ticks: {
            maxTicksLimit: 16,
            autoSkip: true,
            callback: (value) => {
              // value はラベル配列のインデックスとして渡される
              const hz = Number(value) * HZ_PER_BIN;
              return hz >= 1000
                ? `${(hz / 1000).toFixed(1)} kHz`
                : `${hz.toFixed(0)} Hz`;
            },
          },
          grid: {
            color: "rgba(148, 163, 184, 0.15)",
          },
        },
        y: {
          min: FFT_MIN,
          max: FFT_MAX,
          title: {
            display: true,
            text: "Amplitude (0-255)",
          },
          ticks: {
            stepSize: 32,
          },
          grid: {
            color: "rgba(148, 163, 184, 0.2)",
          },
        },
      },
      elements: {
        line: {
          capBezierPoints: false,
        },
      },
    }),
    [],
  );

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let mounted = true;

    const setup = async () => {
      try {
        // 1. listen を登録
        const dispose = await listen<number[]>(FFT_EVENT_NAME, (event) => {
          if (!mounted) return;

          const payload = Array.isArray(event.payload) ? event.payload : [];

          // chartRef 経由で直接 Chart.js インスタンスを更新
          const chart = chartRef.current;
          if (chart) {
            chart.data.datasets[0].data = normalizeFftArray(payload);
            chart.update("none"); // アニメーションなしで即時描画
          }
        });

        if (!mounted) {
          dispose();
          return;
        }

        unlisten = dispose;
        console.log("[fft] listen registered, invoking start_fft_stream");

        // 2. listen 完了後に Rust ストリームを開始
        await invoke("start_fft_stream");
        console.log("[fft] start_fft_stream invoked");
      } catch (err) {
        console.error("[fft] setup error:", err);
      }
    };

    setup();

    return () => {
      mounted = false;
      invoke("stop_fft_stream").catch((e) =>
        console.warn("[fft] stop_fft_stream error:", e),
      );
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  return (
    <main className="container">
      <h1>FFT GUI - 1024bin Line Chart</h1>

      <div
        style={{
          width: "min(1100px, 96vw)",
          height: "460px",
          margin: "0 auto",
          padding: "1rem",
          borderRadius: "12px",
          background: "rgba(255, 255, 255, 0.78)",
          boxShadow: "0 6px 20px rgba(0, 0, 0, 0.08)",
        }}
      >
        <Line ref={chartRef} data={initialData} options={options} />
      </div>
    </main>
  );
}

export default App;
