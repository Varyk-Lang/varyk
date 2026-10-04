# Changelog

## [0.6.0](https://github.com/Varyk-Lang/varyk/compare/varyk-syntax-v0.5.0...varyk-syntax-v0.6.0) (2026-10-04)


### Features

* milestone 5b3, facades for packages ([#28](https://github.com/Varyk-Lang/varyk/issues/28)) ([7dfb049](https://github.com/Varyk-Lang/varyk/commit/7dfb049b76a0803be0f6863ce0bbe69bb2bd6344))

## [0.5.0](https://github.com/Varyk-Lang/varyk/compare/varyk-syntax-v0.4.0...varyk-syntax-v0.5.0) (2026-10-02)


### Miscellaneous Chores

* **varyk-syntax:** Synchronize varyk versions

## [0.4.0](https://github.com/Varyk-Lang/varyk/compare/varyk-syntax-v0.3.0...varyk-syntax-v0.4.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* async and await are keywords, and Task, Shared, and time are reserved names; a program with its own struct named Task must rename it.
* s.parse() returns Result<T, Error> instead of Option<T>; the old Option is `let r: Result<i32, Error> = s.parse();` then `r.ok()`. The names json, env, log, Error, assert, assert_eq, and any item name starting with varyk_ are reserved.
* the public syntax tree of varyk-syntax changed (new expression, statement, and pattern variants, and new fields), so code matching on it must be updated. Varyk programs that milestone 3 accepted are still accepted.
* struct fields are private unless marked `pub`; a milestone-2 program that reads a field across a module boundary must add `pub` to the field.

### Features

* milestone 1, the compiler skeleton and the borrow-by-default proof ([da90c5f](https://github.com/Varyk-Lang/varyk/commit/da90c5f77bf80da14147776d05dc9d0fbd9d9654))
* milestone 2, enums, matching, methods, and collections ([05b742d](https://github.com/Varyk-Lang/varyk/commit/05b742d5790b0196d1ea4e656900381f8fdea5d8))
* milestone 3, packages and interop ([ae30ac9](https://github.com/Varyk-Lang/varyk/commit/ae30ac9c7730651eb2488ece3f0782ba64f2c62c))
* milestone 4, closures, iterators, and patterns ([d336da5](https://github.com/Varyk-Lang/varyk/commit/d336da5c8687af3593867b704957f59c91a5de84))
* milestone 5a, data, config, logging, and tests ([efedf8d](https://github.com/Varyk-Lang/varyk/commit/efedf8d4077704aa11601de1b73f7a7deb753526))
* milestone 5b1, async functions and tasks ([249393e](https://github.com/Varyk-Lang/varyk/commit/249393e0db12a1f37ca0c15513b564d6f03e9ac9))

## [0.3.0](https://github.com/Varyk-Lang/varyk/compare/varyk-syntax-v0.2.0...varyk-syntax-v0.3.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* s.parse() returns Result<T, Error> instead of Option<T>; the old Option is `let r: Result<i32, Error> = s.parse();` then `r.ok()`. The names json, env, log, Error, assert, assert_eq, and any item name starting with varyk_ are reserved.

### Features

* milestone 5a, data, config, logging, and tests ([1fd15b7](https://github.com/Varyk-Lang/varyk/commit/1fd15b7e1fdc865f09532076c502a978e3c7b956))

## [0.2.0](https://github.com/Varyk-Lang/varyk/compare/varyk-syntax-v0.1.0...varyk-syntax-v0.2.0) (2026-09-29)


### ⚠ BREAKING CHANGES

* the public syntax tree of varyk-syntax changed (new expression, statement, and pattern variants, and new fields), so code matching on it must be updated. Varyk programs that milestone 3 accepted are still accepted.

### Features

* milestone 4, closures, iterators, and patterns ([d336da5](https://github.com/Varyk-Lang/varyk/commit/d336da5c8687af3593867b704957f59c91a5de84))

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
