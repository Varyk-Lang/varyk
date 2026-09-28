# Changelog

## [0.1.0](https://github.com/Varyk-Lang/varyk/compare/varyk-syntax-v0.0.2...varyk-syntax-v0.1.0) (2026-09-28)


### ⚠ BREAKING CHANGES

* struct fields are private unless marked `pub`; a milestone-2 program that reads a field across a module boundary must add `pub` to the field.

### Features

* milestone 3, packages and interop ([ae30ac9](https://github.com/Varyk-Lang/varyk/commit/ae30ac9c7730651eb2488ece3f0782ba64f2c62c))

## [0.0.2](https://github.com/Varyk-Lang/varyk/compare/varyk-syntax-v0.0.1...varyk-syntax-v0.0.2) (2026-09-26)


### Features

* milestone 2, enums, matching, methods, and collections ([05b742d](https://github.com/Varyk-Lang/varyk/commit/05b742d5790b0196d1ea4e656900381f8fdea5d8))

## 0.0.1 (2026-09-24)


### Features

* milestone 1, the compiler skeleton and the borrow-by-default proof ([da90c5f](https://github.com/Varyk-Lang/varyk/commit/da90c5f77bf80da14147776d05dc9d0fbd9d9654))
