import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

function manualChunks(id: string) {
  const normalizedId = id.replaceAll("\\", "/");
  if (!normalizedId.includes("/node_modules/")) {
    return undefined;
  }

  if (normalizedId.includes("/@xterm/")) {
    return "vendor-terminal";
  }

  if (
    normalizedId.includes("/react/") ||
    normalizedId.includes("/react-dom/") ||
    normalizedId.includes("/scheduler/")
  ) {
    return "vendor-react";
  }

  if (
    normalizedId.includes("/lucide-react/") ||
    normalizedId.includes("/motion/")
  ) {
    return "vendor-visuals";
  }

  if (
    normalizedId.includes("/@radix-ui/") ||
    normalizedId.includes("/cmdk/") ||
    normalizedId.includes("/class-variance-authority/") ||
    normalizedId.includes("/clsx/") ||
    normalizedId.includes("/tailwind-merge/")
  ) {
    return "vendor-ui";
  }

  return undefined;
}

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  base: "/web/",
  build: {
    target: "esnext",
    outDir: "dist",
    emptyOutDir: true,
    minify: "esbuild",
    chunkSizeWarningLimit: 1000,
    cssCodeSplit: false,
    modulePreload: false,
    rollupOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "assets/[name]-[hash].js",
        manualChunks,
        assetFileNames: (assetInfo) => {
          if (assetInfo.names?.some((name) => name.endsWith(".css"))) return "app.css";
          return "assets/[name]-[hash][extname]";
        },
      },
    },
  },
});
