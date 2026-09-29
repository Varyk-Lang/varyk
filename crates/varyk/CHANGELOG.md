# Changelog

## [0.2.0](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.1.0...varyk-v0.2.0) (2026-09-29)


### ⚠ BREAKING CHANGES

* the public syntax tree of varyk-syntax changed (new expression, statement, and pattern variants, and new fields), so code matching on it must be updated. Varyk programs that milestone 3 accepted are still accepted.

### Features

* milestone 4, closures, iterators, and patterns ([d336da5](https://github.com/Varyk-Lang/varyk/commit/d336da5c8687af3593867b704957f59c91a5de84))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.1.0 to 0.2.0

## [0.1.0](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.0.2...varyk-v0.1.0) (2026-09-28)


### ⚠ BREAKING CHANGES

* struct fields are private unless marked `pub`; a milestone-2 program that reads a field across a module boundary must add `pub` to the field.

### Features

* milestone 3, packages and interop ([ae30ac9](https://github.com/Varyk-Lang/varyk/commit/ae30ac9c7730651eb2488ece3f0782ba64f2c62c))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.0.2 to 0.1.0

## [0.0.2](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.0.1...varyk-v0.0.2) (2026-09-26)


### Features

* milestone 2, enums, matching, methods, and collections ([05b742d](https://github.com/Varyk-Lang/varyk/commit/05b742d5790b0196d1ea4e656900381f8fdea5d8))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.0.1 to 0.0.2

## 0.0.1 (2026-09-24)


### Features

* milestone 1, the compiler skeleton and the borrow-by-default proof ([da90c5f](https://github.com/Varyk-Lang/varyk/commit/da90c5f77bf80da14147776d05dc9d0fbd9d9654))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.0.0 to 0.0.1
