# Changelog

## [0.7.1](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.7.0...varyk-v0.7.1) (2026-10-06)


### Bug Fixes

* varyk init writes a .dockerignore, and a plainer V0219 note ([4327889](https://github.com/Varyk-Lang/varyk/commit/432788903f0034467e98d81ad83fc77b083f6af0))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.7.0 to 0.7.1

## [0.7.0](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.6.0...varyk-v0.7.0) (2026-10-06)


### Features

* milestone 5b4, the compiler side of varyk-http ([4f12606](https://github.com/Varyk-Lang/varyk/commit/4f12606567fbc35a5a89e53b66e530cac9ca8814))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.6.0 to 0.7.0

## [0.6.0](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.5.0...varyk-v0.6.0) (2026-10-04)


### Features

* milestone 5b3, facades for packages ([#28](https://github.com/Varyk-Lang/varyk/issues/28)) ([7dfb049](https://github.com/Varyk-Lang/varyk/commit/7dfb049b76a0803be0f6863ce0bbe69bb2bd6344))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.5.0 to 0.6.0

## [0.5.0](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.4.0...varyk-v0.5.0) (2026-10-02)


### ⚠ BREAKING CHANGES

* a package's Cargo.toml must name its .vr root ([[bin]] with path src/main.vr, or [lib] with path src/lib.vr), or varyk check reports V0406; varyk emit is removed; varyk init no longer writes build.rs or a stub, and a source package is built with varyk build, not plain cargo build; build may only be left out or false.

### Features

* milestone 5b2, packages ([6c6df21](https://github.com/Varyk-Lang/varyk/commit/6c6df21fe1289ba3e45ae7c6e64d3a20e9f5beb4))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.4.0 to 0.5.0

## [0.4.0](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.3.0...varyk-v0.4.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* async and await are keywords, and Task, Shared, and time are reserved names; a program with its own struct named Task must rename it.

### Features

* milestone 5b1, async functions and tasks ([249393e](https://github.com/Varyk-Lang/varyk/commit/249393e0db12a1f37ca0c15513b564d6f03e9ac9))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.3.0 to 0.4.0

## [0.3.0](https://github.com/Varyk-Lang/varyk/compare/varyk-v0.2.0...varyk-v0.3.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* s.parse() returns Result<T, Error> instead of Option<T>; the old Option is `let r: Result<i32, Error> = s.parse();` then `r.ok()`. The names json, env, log, Error, assert, assert_eq, and any item name starting with varyk_ are reserved.

### Features

* milestone 5a, data, config, logging, and tests ([1fd15b7](https://github.com/Varyk-Lang/varyk/commit/1fd15b7e1fdc865f09532076c502a978e3c7b956))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * varyk-syntax bumped from 0.2.0 to 0.3.0

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
