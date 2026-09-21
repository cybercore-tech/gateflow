# gateflow mockup page

This directory contains the static GitHub Pages mockup for gateflow. It is intentionally separate from the generated Rust documentation under `docs/`.

The repository workflow at `.github/workflows/pages.yml` publishes this directory when changes land on `main`. In the repository settings, set **Pages → Build and deployment → Source** to **GitHub Actions** once.

The page is dependency-free at runtime. It uses Google Fonts when available and falls back to system fonts when the page is viewed offline.
