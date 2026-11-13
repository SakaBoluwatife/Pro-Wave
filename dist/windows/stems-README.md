# Stem Splitter files for the installer

`dist/windows/stems/` is not in git. Before building the installer it must hold:

- `htdemucs_fp16weights.onnx`: the 166 MB model, from https://huggingface.co/StemSplitio/htdemucs-onnx/resolve/main/htdemucs_fp16weights.onnx (SHA-256 `d05c269d0178d2a72ad484b10b11dd370193fc923201c3b27a99f848745db70a`).
- `htdemucs_6s_fp16weights.onnx`: the 136 MB model for Guitar and Piano, from https://huggingface.co/StemSplitio/htdemucs-6s-onnx/resolve/main/htdemucs_6s_fp16weights.onnx (SHA-256 `7ce55792e2231c93fbf92de95f5fd5b3a5e6c89f7db690dfd693e8f1dce56869`).
- `drumsep_v2_w10_f16w.onnx`: the 168 MB model for the pieces of the kit, from https://huggingface.co/b33zy/drumsep-v2-onnx/resolve/main/drumsep_v2_w10_f16w.onnx (SHA-256 `b05779ea756870f3fe359c63ac501dd12caf3b24d66237b1404bb4dd5b32839a`). MIT, from inagoy/drumsep by way of gridshiftstudio/drumsep-onnx. Its card says the training data of the first version is not documented.
- `onnxruntime.dll`, `onnxruntime_providers_shared.dll`, `ONNXRUNTIME-LICENSE` and `ONNXRUNTIME-ThirdPartyNotices.txt`: from `lib/` and the top of https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-1.31.1.zip. MIT. The version must suit the `ort` crate in `crates/ovrtone-app/Cargo.toml`.

**Model licence:** unresolved. The model's page and the Demucs code say MIT, but no explicit licence from Meta for the trained weights was found, and reports quote a project statement that they were for research use. The owner chose to bundle them while Ovrtone is not sold. Revisit this before any sale, or download the model on first use, or replace it with one of our own.

The splitter runs on the CPU, in a copy of `ovrtone.exe` started with `--split-stems`.

For a development build, set `OVRTONE_STEMS_DIR` to a folder holding these files.
