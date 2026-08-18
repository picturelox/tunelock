import type { Config } from "tailwindcss";

const config: Config = {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        background: "#0f0f0f",
        surface: "#1a1a2e",
        "surface-light": "#252542",
        "accent-primary": "#e94560",
        "accent-secondary": "#0f3460",
        "text-primary": "#eaeaea",
        "text-secondary": "#7a7a7a",
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "sans-serif"],
      },
    },
  },
  plugins: [],
};

export default config;
