# tauri-pilot Documentation

This directory contains the documentation site for [tauri-pilot](https://github.com/mpiton/tauri-pilot), built with [Astro Starlight](https://starlight.astro.build).

## Development

```bash
npm install
npm run dev       # Start dev server at localhost:4321
npm run build     # Build for production to ./dist/
npm run preview   # Preview production build locally
```

## Structure

```text
src/
├── assets/           # Images and logos
├── content/
│   └── docs/         # Markdown documentation pages
│       ├── guides/   # Technical guides
│       └── reference/ # CLI reference
├── styles/           # Custom CSS
└── content.config.ts # Content schema
```

Documentation pages are `.md` files in `src/content/docs/`. Each file is exposed as a route based on its file name.

## Deployment

The site is automatically deployed to GitHub Pages on push to `main` via the `deploy-docs.yml` workflow.
