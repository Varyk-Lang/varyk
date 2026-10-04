# Changelog

## [0.6.0](https://github.com/Varyk-Lang/varyk/compare/varyk-std-v0.5.0...varyk-std-v0.6.0) (2026-10-04)


### Features

* milestone 5b3, facades for packages ([#28](https://github.com/Varyk-Lang/varyk/issues/28)) ([7dfb049](https://github.com/Varyk-Lang/varyk/commit/7dfb049b76a0803be0f6863ce0bbe69bb2bd6344))

## [0.5.0](https://github.com/Varyk-Lang/varyk/compare/varyk-std-v0.4.0...varyk-std-v0.5.0) (2026-10-02)


### Miscellaneous Chores

* **varyk-std:** Synchronize varyk versions

## [0.4.0](https://github.com/Varyk-Lang/varyk/compare/varyk-std-v0.3.0...varyk-std-v0.4.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* async and await are keywords, and Task, Shared, and time are reserved names; a program with its own struct named Task must rename it.
* s.parse() returns Result<T, Error> instead of Option<T>; the old Option is `let r: Result<i32, Error> = s.parse();` then `r.ok()`. The names json, env, log, Error, assert, assert_eq, and any item name starting with varyk_ are reserved.

### Features

* milestone 5a, data, config, logging, and tests ([efedf8d](https://github.com/Varyk-Lang/varyk/commit/efedf8d4077704aa11601de1b73f7a7deb753526))
* milestone 5b1, async functions and tasks ([249393e](https://github.com/Varyk-Lang/varyk/commit/249393e0db12a1f37ca0c15513b564d6f03e9ac9))

## [0.3.0](https://github.com/Varyk-Lang/varyk/compare/varyk-std-v0.2.0...varyk-std-v0.3.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* s.parse() returns Result<T, Error> instead of Option<T>; the old Option is `let r: Result<i32, Error> = s.parse();` then `r.ok()`. The names json, env, log, Error, assert, assert_eq, and any item name starting with varyk_ are reserved.

### Features

* milestone 5a, data, config, logging, and tests ([1fd15b7](https://github.com/Varyk-Lang/varyk/commit/1fd15b7e1fdc865f09532076c502a978e3c7b956))
