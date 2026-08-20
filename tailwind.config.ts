import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // Data plane (charcoal, unchanged) — used for waveforms, tables, timing
        background: "#0f0f0f",
        surface: "#1a1a1e",
        "surface-light": "#222226",
        "data-bg": "#1a1a1e",
        "data-surface": "#222226",
        "data-text": "#e8e8ec",
        "data-text-dim": "#888892",
        "data-border": "#333338",
        "data-grid": "#2a2a2e",

        // Walnut Console frame tokens
        "walnut-base": "#3d2b1f",
        "walnut-dark": "#2a1e15",
        "walnut-light": "#4a3829",
        "bronze-face": "#8b7355",
        "bronze-dark": "#5c4a35",
        "brass-accent": "#c9a96e",
        "brass-bright": "#e8c87a",
        "cream-label": "#f0e6d2",

        // Semantic state colors
        "lamp-amber": "#d4a04c",
        "lamp-green": "#5c9c5c",
        "lamp-red": "#c45c5c",

        // Legacy accent (being phased out in favor of brass/bronze)
        "accent-primary": "#c9a96e",
        "accent-secondary": "#5c4a35",
        "text-primary": "#eaeaea",
        "text-secondary": "#7a7a7a",
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono", "Consolas", "monospace"],
      },
      boxShadow: {
        'inset-bezel': 'inset 0 1px 2px rgba(0,0,0,0.5), inset 0 -1px 1px rgba(255,255,255,0.05)',
        'bronze-plate': '0 1px 3px rgba(0,0,0,0.4), inset 0 1px 0 rgba(255,255,255,0.1)',
        'slot-glow': '0 0 8px rgba(201,169,110,0.3)',
      },
    },
  },
  plugins: [],
};

export default config;
