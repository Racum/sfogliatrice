# Change Log

All notable changes to this project will be documented in this file.
This project adheres to [Semantic Versioning](http://semver.org/).

## [0.6.0] - 2026-04-30

### Added

- Support to Polygon holes.

## [0.5.1] - 2026-04-28

### Fixed

Fix unneeded convex_hull

## [0.5.0] - 2026-04-28

### Added

- Brute-force option.

# [0.4.2] - 2026-04-26

### Changed

- Optimized WebAssembly output file size.

## [0.4.1] - 2026-04-25

### Changed

- Refactored GeoJSON handling.

## [0.4.0] - 2026-04-25

### Added

- WebAssembly Package.

### Changed

- Intermediates threshold inflation now dynamic.

## [0.3.0] - 2026-04-24

### Added

- Library: "parallel" feature triggers rayon (opt-in).
- CLI: sets parallel feature.

### Changed

- Intermediates logic based on Flatbush and Concave Hull algorithms.

## [0.2.0] - 2026-04-22

### Added

- Added custom heading angle.
- Moved GeoJSON handling to the library core.

## [0.1.0] - 2026-04-21

### Added

- Initial commit, basic tessellation library and CLI tool.
