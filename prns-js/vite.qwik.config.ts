import { qwikVite } from "@builder.io/qwik/optimizer";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [qwikVite({
    srcDir: "src/browser/adapters",
    client: { input: "src/browser/adapters/qwik.tsx" },
  })],
  build: {
    target: "es2022",
    emptyOutDir: false,
    outDir: "dist/qwik",
    lib: {
      entry: "src/browser/adapters/qwik.tsx",
      formats: ["es"],
      fileName: () => "index.qwik.mjs",
    },
    rollupOptions: {
      external: ["@builder.io/qwik", "@builder.io/qwik/jsx-runtime", "personal-rns/browser"],
      plugins: [{
        name: "verify-external-frameworks",
        generateBundle(_options, bundle) {
          for (const chunk of Object.values(bundle)) {
            if (chunk.type !== "chunk") continue;
            for (const [id, module] of Object.entries(chunk.modules)) {
              if (module.renderedLength && id.split(/[\\/]/).includes("node_modules")) {
                this.error(`Unreviewed npm code bundled in the Qwik facade: ${id}`);
              }
            }
          }
        },
      }],
    },
  },
});
