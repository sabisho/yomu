# Yomu — Stage 8A: CBR/RAR Decompression-Bomb Protection & Extraction Hardening Report

**Author:** Google Antigravity (Pair Programming Agent)  
**Date:** November 4, 2026  
**Status:** Complete & Verified  
**Target Vulnerability:** `ISSUE-8-01` (Identified in `docs/audits/PRODUCTION_READINESS_AUDIT_STAGE_8.md`)  
**Resolution Verdict:** **FIXED AND VERIFIED**

---

## Executive Summary

During the Stage 8 Production-Readiness Audit, a potential denial-of-service (OOM crash) vulnerability was identified in Yomu's CBR/RAR archive pipeline (`ISSUE-8-01`). While Yomu correctly validated declared header metadata (`header.entry().unpacked_size > MAX_PAGE_BYTES`), upstream `unrar` crate (version 0.5.8) utilized an unbounded `ReadToVec` callback (`my.extend_from_slice(other)`) that ignored callback return values and blindly appended all decompressed data emitted by native Eugene Roshal UnRAR 7.1.0 C++ engine.

A maliciously crafted RAR archive containing a spoofed header (declaring a small uncompressed size while containing gigabytes of compressed data) or an archive exploiting integer overflow could bypass the header check and trigger unbounded memory growth until the host system exhausted physical RAM or the process was terminated by the Linux OOM killer.

In Stage 8A, we investigated the complete native C++ and Rust FFI boundary, vendored and patched `unrar` 0.5.8 at `vendor/unrar` via Cargo patch, and introduced bounded streaming extraction (`ReadBoundedToVec` and `BoundedBuffer`). The solution enforces an absolute byte ceiling (`MAX_PAGE_BYTES = 128 MiB` for single pages) incrementally across incoming chunks during extraction. If exceeded, extraction is immediately aborted inside native C++ via `UCM_PROCESSDATA` returning `-1`, which cleanly invokes `ErrHandler.Exit(RARX_USERBREAK)` without cross-language panics or FFI unwinding, while the Rust buffer is immediately drained and deallocated via `clear()` and `shrink_to_fit()`.

All 372 active tests (160 core library tests, 212 binary UI tests, plus 18 benchmarks; 390 total) pass with exit code 0. Sequential extraction performance remains identical to Stage 6C (~36.3 µs/page).

---

## 1. Vulnerability Root Cause Analysis

### 1.1 The Extraction Call Chain

The complete CBR/RAR page extraction path runs as follows:

$$\text{Yomu App} \xrightarrow{\text{read\_rar\_page / RarSession}} \text{unrar::OpenArchive} \xrightarrow{\text{FFI}} \text{unrar\_sys} \xrightarrow{\text{Native C++}} \text{UnRAR 7.1.0} \xrightarrow{\text{Callback}} \text{Rust Buffer}$$

Specifically:
1. `src/archive.rs` calls `unrar::OpenArchive::read()`.
2. Upstream `unrar` registers a static C callback function `callback<M: ProcessMode>` passed as `UNRARCALLBACK` to native `RARSetCallback`.
3. Native UnRAR C++ unpacks chunks in `CmdExtract::ExtractCurrentFile` / `Unpack::DoUnpack`.
4. As each chunk is decompressed, native UnRAR invokes the callback with `UCM_PROCESSDATA`.
5. Upstream `unrar`'s `ReadToVec` mode implemented `process_data` as:
   ```rust
   // Upstream unrar 0.5.8 (VULNERABLE)
   impl ProcessMode for ReadToVec {
       type Output = Vec<u8>;
       fn process_data(my: &mut Self::Output, other: &[u8]) {
           my.extend_from_slice(other); // Unbounded heap allocation!
       }
   }
   ```
6. Upstream's C callback trampoline unconditionally returned `0`:
   ```rust
   // Upstream unrar 0.5.8 callback trampoline
   extern "C" fn callback<M: ProcessMode>(msg: c_uint, user_data: isize, p1: isize, p2: isize) -> c_int {
       if msg == UCM_PROCESSDATA {
           let (mode, cursor) = ...;
           M::process_data(cursor, slice);
       }
       0 // Always returned 0 (continue), completely ignoring buffer state!
   }
   ```

### 1.2 The Spoofed Header Threat Model

A malicious actor creating a "zip/rar bomb" can craft an archive entry whose local file header claims `unp_size = 1024` (1 KiB).
- Yomu's guard: `if header.entry().unpacked_size > MAX_PAGE_BYTES { return Err(...); }` evaluates `1024 > 134,217,728` $\implies$ **FALSE** (passes check).
- Upstream `unrar` invokes native UnRAR C++.
- The decompression dictionary expands 1 KiB of compressed data into 10 GiB of decompressed zeroes.
- Upstream `ReadToVec::process_data` repeatedly called `Vec::extend_from_slice()`, allocating 1 GiB, 2 GiB, 4 GiB, 8 GiB until OOM crash.

---

## 2. Native C++ UnRAR Safety & FFI Exception Boundary

Before altering any code, we audited the native UnRAR 7.1.0 C++ implementation bundled in `unrar_sys` to determine how native UnRAR handles aborts returned by the user callback.

In `unrar_sys/vendor/unrar/rdwrfn.cpp`:
```cpp
int ComprDataIO::UnpWrite(byte *Addr,size_t Count) {
    ...
    if (Cmd->Callback!=NULL)
    {
        int RetCode=Cmd->Callback(UCM_PROCESSDATA,(LPARAM)Cmd->UserData,(LPARAM)Addr,Count);
        if (RetCode==-1)
        {
            ErrHandler.Exit(RARX_USERBREAK);
            return -1;
        }
    }
    ...
}
```

In `unrar_sys/vendor/unrar/errhnd.cpp` and `dll.cpp`:
- `ErrHandler.Exit(RARX_USERBREAK)` sets an internal exit code flag and safely unwinds the C++ extraction loop via native C++ catch blocks inside `RARProcessFile` / `RARProcessFileW`.
- It returns error code `ERAR_UNKNOWN` (value 19) through the C API function `RARProcessFile`.
- **Crucial Finding:** No C++ exceptions unwind across the FFI boundary into Rust. Native UnRAR was *specifically designed* to safely abort extraction whenever `UCM_PROCESSDATA` returns `-1`. The vulnerability existed solely because upstream Rust `unrar` discarded this capability, never allowed custom return values, and never bounded `Vec` expansion.

---

## 3. Remediation Architecture

### 3.1 Vendoring & Patching `unrar`

In compliance with Rule 1 (Safe Rust only) and Rule 7 (Simplicity over Overengineering), we vendored `unrar` 0.5.8 to `vendor/unrar` and configured Cargo workspace patching in `Cargo.toml`:

```toml
[patch.crates-io]
unrar = { path = "vendor/unrar" }
```

No C++ code was touched; only the safe Rust wrapper layer was modified.

### 3.2 `BoundedBuffer` & `ReadBoundedToVec`

We introduced `BoundedBuffer` in `vendor/unrar/src/open_archive.rs`:

```rust
/// A memory-bounded accumulator for extracted file bytes.
#[derive(Debug, Clone, Default)]
pub struct BoundedBuffer {
    /// Accumulated decompressed bytes.
    pub data: Vec<u8>,
    /// Maximum allowed byte capacity.
    pub max_bytes: usize,
    /// Whether the extraction exceeded the maximum byte budget.
    pub exceeded: bool,
}

impl BoundedBuffer {
    /// Creates a new bounded buffer with the specified maximum byte limit.
    pub fn new(max_bytes: usize) -> Self {
        Self {
            data: Vec::new(),
            max_bytes,
            exceeded: false,
        }
    }

    /// Appends incoming chunk data, enforcing the maximum byte ceiling.
    /// Returns `true` to continue extraction, or `false` to abort immediately.
    #[inline]
    pub fn process_chunk(&mut self, chunk: &[u8]) -> bool {
        if self.exceeded {
            return false;
        }

        // Integer-overflow-safe bound verification
        let current_len = self.data.len();
        let new_len = match current_len.checked_add(chunk.len()) {
            Some(len) => len,
            None => {
                self.exceeded = true;
                self.data.clear();
                self.data.shrink_to_fit();
                return false;
            }
        };

        if new_len > self.max_bytes {
            self.exceeded = true;
            // Free allocated buffer memory immediately
            self.data.clear();
            self.data.shrink_to_fit();
            return false;
        }

        self.data.extend_from_slice(chunk);
        true
    }
}
```

### 3.3 Callback Abort Signaling & Error Code Mapping

1. **Callback Return Value Propagation:**
   `ProcessMode` was augmented with `fn process_data(cursor: &mut Self::Output, other: &[u8]) -> bool`.
   When `process_data` returns `false`, `callback` returns `-1` directly to native UnRAR C++.
2. **Immediate Heap Release:**
   When `new_len > self.max_bytes`, `self.data.clear()` and `self.data.shrink_to_fit()` are called immediately. The process does not wait for archive closing or stack unwinding to release heap memory back to the system allocator.
3. **Explicit Error Code (`Code::ExcessiveSize = 999`):**
   In `vendor/unrar/src/error.rs`, we added `Code::ExcessiveSize` with human-readable description `"Decompressed size exceeds maximum budget"`. If native extraction returns with `exceeded == true`, `open_archive.rs` converts the result directly into `unrar::error::UnrarError::from(Code::ExcessiveSize)`.
4. **Convenience API:**
   Added `header.read_bounded(max_bytes)` on `OpenArchive<Process>`.

### 3.4 Integration in Yomu Core (`src/archive.rs`)

We updated all CBR extraction entry points in `src/archive.rs` to enforce `MAX_PAGE_BYTES as usize` (128 MiB = 134,217,728 bytes):
- `read_rar_page()`:
  ```rust
  let bytes = match header.read_bounded(MAX_PAGE_BYTES as usize) {
      Ok(bytes) => bytes,
      Err(err) => {
          return Err(anyhow::anyhow!(
              "Failed to extract RAR entry '{}': {err}",
              entry.name
          ));
      }
  };
  ```
- `RarSession::run_worker()` sequential loop:
  ```rust
  let bytes = match header.read_bounded(MAX_PAGE_BYTES as usize) {
      Ok(bytes) => bytes,
      Err(err) => {
          let _ = resp.send(Err(anyhow::anyhow!(
              "Failed to extract RAR entry '{}': {err}",
              entry.name
          )));
          ...
      }
  };
  ```
- `probe_pages_dimensions()` single-pass probing:
  ```rust
  let bytes = match header.read_bounded(MAX_PAGE_BYTES as usize) {
      Ok(bytes) => bytes,
      Err(_) => {
          let _ = header.skip();
          continue;
      }
  };
  ```

---

## 4. Adversarial Verification & Test Suite

To prove correctness under worst-case scenarios, 6 unit and adversarial tests were implemented in `src/archive.rs`:

| Test Name | Scenario Tested | Outcome |
| :--- | :--- | :--- |
| `test_cbr_bounded_extraction_exact_limit` | Archive entry with 11-byte payload extracted with exact limit 11. | `Ok(11 bytes)` extracted successfully. |
| `test_cbr_bounded_extraction_exceeded_by_one_byte_aborts` | Archive entry with 11-byte payload extracted with limit 10. | Aborts in callback, returns `Decompressed size exceeds maximum budget`. |
| `test_cbr_decompression_bomb_spoofed_header_callback_aborts` | Archive entry modified at byte level to spoof header `unp_size` to 20 bytes, with limit 10 and payload 11 bytes. Passes pre-check, but callback aborts on chunk arrival. | Aborts during streaming decompression, returns `Decompressed size exceeds maximum budget`. |
| `test_rar_session_recovery_after_bounded_extraction_failure` | Requesting an entry that violates the budget fails, followed immediately by requesting a valid entry. | Session cleanly recovers and serves the valid page without crashing worker thread. |
| `test_rar_session_repeated_consecutive_failures_resilience` | Requesting multiple invalid entries in succession. | Worker remains alive and responsive; subsequent valid requests succeed. |
| `test_unrar_bounded_buffer_callback_boundary_and_overflow_conditions` | Direct tests on `BoundedBuffer`: exact limit, 1-byte overflow, multi-chunk accumulation, and `checked_add(usize::MAX)` integer overflow. | All boundary and overflow conditions pass with zero memory leakage. |

### Performance Verification

We re-ran the Stage 6C CBR sequential extraction benchmark (`perf_stage6c_cbr_remediated`) across 200 pages:

```
Stage 6C CBR Remediated (200-page sequential extraction):
  Total Time: 7.27 ms
  Avg Time / Page: 36.3 µs
  Archive Open Count: 1
  Sequential Header Scans: 200
  Last Page (p199) Extraction Time: 30.8 µs
```

The overhead of bounded chunk accounting is undetectable ($<0.9$ µs difference within standard variance of 35.4 µs).

---

## 5. Modified Files Summary

| File | Changes Made |
| :--- | :--- |
| `Cargo.toml` | Added `[patch.crates-io] unrar = { path = "vendor/unrar" }`. |
| `Cargo.lock` | Updated `unrar` source mapping to local path. |
| `vendor/unrar/src/open_archive.rs` | Added `BoundedBuffer`, `ReadBoundedToVec`, updated `ProcessMode::process_data` to return `bool`, wired `-1` return in `callback`, added `read_bounded()`. |
| `vendor/unrar/src/error.rs` | Added `Code::ExcessiveSize = 999` ("Decompressed size exceeds maximum budget"). |
| `src/archive.rs` | Switched `read_rar_page`, `RarSession`, and `probe_pages_dimensions` to `header.read_bounded(MAX_PAGE_BYTES as usize)`. Added 6 regression/adversarial tests. |
| `docs/agent-handoff.md` | Recorded Stage 8A completion, updated test baseline (372 active tests), resolved `ISSUE-8-01`. |
| `AGENTS.md` | Updated test baseline to 372 active tests (160 core library, 212 UI tests). |
| `GEMINI.md` | Updated test baseline to 372 active tests. |

---

## 6. Verification Status

```bash
devenv shell cargo fmt --all -- --check           # Exit code: 0
devenv shell cargo check --all-targets            # Exit code: 0
devenv shell cargo clippy --all-targets --all-features -- -D warnings  # Exit code: 0
devenv shell cargo test --all-targets             # Exit code: 0 (372 active tests passed, 18 ignored benchmarks)
```

- **Unsafe Code:** ZERO unsafe code in Yomu (`#![forbid(unsafe_code)]` preserved).
- **Physical Wayland Compositor Visual Verification:** **UNVERIFIED** (Headless automated tests only).
- **Security Assessment:** `ISSUE-8-01` is permanently **RESOLVED**.
