import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        // === API 2500 Console Palette ===
        // Faceplate: blue-grey steel
        'plate-base': '#4a5a6a',
        'plate-dark': '#3a4a5a',
        'plate-light': '#5a6b7a',
        'plate-darker': '#2a3a4a',

        // Channel strip body
        'strip-bg': '#3a4a5a',
        'strip-dark': '#2a3a4a',
        'strip-light': '#4a5a6a',

        // Knobs and faders
        'knob-body': '#1a1a1e',
        'knob-rim': '#3a3a3e',
        'knob-indicator': '#d4a04c',
        'fader-cap': '#e8e8ec',
        'fader-track': '#1a1a1e',

        // Button caps — amber/orange (API signature)
        'cap-amber': '#d4a04c',
        'cap-amber-bright': '#e8b85c',
        'cap-amber-dark': '#a07830',
        'cap-red': '#c45c3c',
        'cap-green': '#5c8a5c',
        'cap-white': '#e8e8ec',
        'cap-black': '#1a1a1e',

        // VU meter
        'vu-bg': '#1a2a1a',
        'vu-needle': '#d4a04c',
        'vu-scale': '#c0c0a0',
        'vu-red': '#c45c3c',
        'vu-green': '#5c8a5c',
        'vu-amber': '#d4a04c',

        // Labels and text
        'label-cream': '#e0d8c0',
        'label-bright': '#f0e6d2',
        'label-dim': '#8a8a7a',

        // Data plane (waveforms, analysis — stays dark)
        'data-bg': '#0f0f0f',
        'data-surface': '#1a1a1e',
        'data-text': '#e8e8ec',
        'data-text-dim': '#6a6a6a',
        'data-border': '#2a2a2e',

        // Background (behind the console)
        'background': '#1a1a1e',
        'surface': '#2a2a2e',

        // Legacy compat (mapped to new palette)
        'accent-primary': '#d4a04c',
        'accent-secondary': '#3a4a5a',
        'text-primary': '#e0d8c0',
        'text-secondary': '#8a8a7a',
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono", "Consolas", "monospace"],
      },
      boxShadow: {
        'plate-bezel': 'inset 0 1px 2px rgba(0,0,0,0.6), inset 0 -1px 1px rgba(255,255,255,0.08)',
        'knob-3d': 'inset 0 2px 3px rgba(0,0,0,0.7), 0 1px 0 rgba(255,255,255,0.05)',
        'cap-3d': '0 1px 2px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.15)',
        'cap-pressed': 'inset 0 1px 3px rgba(0,0,0,0.6)',
        'vu-glow': '0 0 8px rgba(212,160,76,0.15), inset 0 0 12px rgba(0,0,0,0.4)',
        'slot-inset': 'inset 0 2px 4px rgba(0,0,0,0.5)',
      },
    },
  },
  plugins: [],
};

export default config;
