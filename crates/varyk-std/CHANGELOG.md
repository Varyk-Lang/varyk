# Changelog

## [0.4.0](https://github.com/Varyk-Lang/varyk/compare/varyk-std-v0.3.0...varyk-std-v0.4.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* s.parse() returns Result<T, Error> instead of Option<T>; the old Option is `let r: Result<i32, Error> = s.parse();` then `r.ok()`. The names json, env, log, Error, assert, assert_eq, and any item name starting with varyk_ are reserved.

### Features

* milestone 5a, data, config, logging, and tests ([efedf8d](https://github.com/Varyk-Lang/varyk/commit/efedf8d4077704aa11601de1b73f7a7deb753526))

## [0.3.0](https://github.com/Varyk-Lang/varyk/compare/varyk-std-v0.2.0...varyk-std-v0.3.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* s.parse() returns Result<T, Error> instead of Option<T>; the old Option is `let r: Result<i32, Error> = s.parse();` then `r.ok()`. The names json, env, log, Error, assert, assert_eq, and any item name starting with varyk_ are reserved.

### Features

* milestone 5a, data, config, logging, and tests ([1fd15b7](https://github.com/Varyk-Lang/varyk/commit/1fd15b7e1fdc865f09532076c502a978e3c7b956))
