/// cbindgen:ignore
mod compatible_engine;
mod helpers;
use self::helpers::*;
use libc::c_void;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::mem::size_of;
use std::os::raw::c_char;
use std::ptr::null;
use std::sync::{Mutex, MutexGuard};
use std::{env, io};
use tracing_subscriber::EnvFilter;
use voicevox_core::AudioQueryModel;
use voicevox_core::Result;
use voicevox_core::VoicevoxCore;

#[cfg(test)]
use rstest::*;

type Internal = VoicevoxCore;

static INTERNAL: Lazy<Mutex<Internal>> = Lazy::new(|| {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(if env::var_os(EnvFilter::DEFAULT_ENV).is_some() {
            EnvFilter::from_default_env()
        } else {
            "error,voicevox_core=info,voicevox_core_c_api=info,onnxruntime=info".into()
        })
        .with_writer(io::stderr)
        .try_init();

    Internal::new_with_mutex()
});

pub(crate) fn lock_internal() -> MutexGuard<'static, Internal> {
    INTERNAL.lock().unwrap()
}

static INTERNAL_BUFFER_VEC_LENGTH: Lazy<Mutex<HashMap<usize, usize>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

// Vecのポインタとサイズを分解して取得する関数
// 取り回しとして，INTERNAL_BUFFER_VEC_LENGTHにサイズを保存する
fn leak_vec_and_store_length<T>(mut vec: Vec<T>) -> (*mut T, usize) {
    assert!(
        size_of::<T>() >= 1,
        "サイズが0の値のVecはコーナーケースになりやすいためエラーにする"
    );

    let size = vec.len();
    vec.shrink_to_fit();
    let ptr = vec.leak().as_ptr();
    let addr = ptr as usize;

    let not_occupied = INTERNAL_BUFFER_VEC_LENGTH
        .lock()
        .unwrap()
        .insert(addr, size)
        .is_none();

    assert!(not_occupied, "すでに値が入っている状態はおかしい");

    (ptr as *mut T, size)
}

// ポインタからVecを復元する関数
// 引数に渡されるポインタは`vec_leak_with_size`で作られた必要がある
unsafe fn restore_vec<T>(buffer_ptr: *const T) -> Vec<T> {
    let addr = buffer_ptr as usize;
    let size = INTERNAL_BUFFER_VEC_LENGTH
        .lock()
        .unwrap()
        .remove(&addr)
        .expect("管理されていないポインタを渡した");

    Vec::from_raw_parts(buffer_ptr as *mut T, size, size)
}

/*
 * Cの関数として公開するための型や関数を定義するこれらの実装はvoicevox_core/publish.rsに定義してある対応する関数にある
 * この関数ではvoicevox_core/publish.rsにある対応する関数の呼び出しと、その戻り値をCの形式に変換する処理のみとする
 * これはC文脈の処理と実装をわけるためと、内部実装の変更がAPIに影響を与えにくくするためである
 * voicevox_core/publish.rsにある対応する関数とはこのファイルに定義してある公開関数からvoicevoxプレフィックスを取り除いた名前の関数である
 */

pub use voicevox_core::result_code::VoicevoxResultCode;

/// ハードウェアアクセラレーションモードを設定する設定値
#[repr(i32)]
#[derive(Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum VoicevoxAccelerationMode {
    /// 実行環境に合った適切なハードウェアアクセラレーションモードを選択する
    VOICEVOX_ACCELERATION_MODE_AUTO = 0,
    /// ハードウェアアクセラレーションモードを"CPU"に設定する
    VOICEVOX_ACCELERATION_MODE_CPU = 1,
    /// ハードウェアアクセラレーションモードを"GPU"に設定する
    VOICEVOX_ACCELERATION_MODE_GPU = 2,
}

/// 初期化オプション
#[repr(C)]
pub struct VoicevoxInitializeOptions {
    /// ハードウェアアクセラレーションモード
    acceleration_mode: VoicevoxAccelerationMode,
    /// CPU利用数を指定
    /// 0を指定すると環境に合わせたCPUが利用される
    cpu_num_threads: u16,
    /// 全てのモデルを読み込む
    load_all_models: bool,
    /// open_jtalkの辞書ディレクトリ
    open_jtalk_dict_dir: *const c_char,
}

/// デフォルトの初期化オプションを生成する
/// @return デフォルト値が設定された初期化オプション
#[no_mangle]
pub extern "C" fn voicevox_make_default_initialize_options() -> VoicevoxInitializeOptions {
    VoicevoxInitializeOptions::default()
}

/// 初期化する
/// @param [in] options 初期化オプション
/// @return 結果コード #VoicevoxResultCode
#[no_mangle]
pub extern "C" fn voicevox_initialize(options: VoicevoxInitializeOptions) -> VoicevoxResultCode {
    into_result_code_with_error((|| {
        let options = unsafe { options.try_into_options() }?;
        lock_internal().initialize(options)?;
        Ok(())
    })())
}

static VOICEVOX_VERSION: once_cell::sync::Lazy<CString> =
    once_cell::sync::Lazy::new(|| CString::new(Internal::get_version()).unwrap());

/// voicevoxのバージョンを取得する
/// @return SemVerでフォーマットされたバージョン
#[no_mangle]
pub extern "C" fn voicevox_get_version() -> *const c_char {
    VOICEVOX_VERSION.as_ptr()
}

/// モデルを読み込む
/// @param [in] speaker_id 読み込むモデルの話者ID
/// @return 結果コード #VoicevoxResultCode
#[no_mangle]
pub extern "C" fn voicevox_load_model(speaker_id: u32) -> VoicevoxResultCode {
    into_result_code_with_error(lock_internal().load_model(speaker_id).map_err(Into::into))
}

/// ハードウェアアクセラレーションがGPUモードか判定する
/// @return GPUモードならtrue、そうでないならfalse
#[no_mangle]
pub extern "C" fn voicevox_is_gpu_mode() -> bool {
    lock_internal().is_gpu_mode()
}

/// 指定したspeaker_idのモデルが読み込まれているか判定する
/// @return モデルが読み込まれているのであればtrue、そうでないならfalse
#[no_mangle]
pub extern "C" fn voicevox_is_model_loaded(speaker_id: u32) -> bool {
    lock_internal().is_model_loaded(speaker_id)
}

/// このライブラリの利用を終了し、確保しているリソースを解放する
#[no_mangle]
pub extern "C" fn voicevox_finalize() {
    lock_internal().finalize()
}

/// メタ情報をjsonで取得する
/// @return メタ情報のjson文字列
#[no_mangle]
pub extern "C" fn voicevox_get_metas_json() -> *const c_char {
    lock_internal().get_metas_json().as_ptr()
}

/// サポートデバイス情報をjsonで取得する
/// @return サポートデバイス情報のjson文字列
#[no_mangle]
pub extern "C" fn voicevox_get_supported_devices_json() -> *const c_char {
    lock_internal().get_supported_devices_json().as_ptr()
}

/// 音素ごとの長さを推論する
/// @param [in] length phoneme_vector, output のデータ長
/// @param [in] phoneme_vector  音素データ
/// @param [in] speaker_id 話者ID
/// @param [out] output_predict_duration_length 出力データのサイズ
/// @param [out] output_predict_duration_data データの出力先
/// @return 結果コード #VoicevoxResultCode
///
/// # Safety
/// @param phoneme_vector 必ずlengthの長さだけデータがある状態で渡すこと
/// @param output_predict_duration_data_length uintptr_t 分のメモリ領域が割り当てられていること
/// @param output_predict_duration_data 成功後にメモリ領域が割り当てられるので ::voicevox_predict_duration_data_free で解放する必要がある
#[no_mangle]
pub unsafe extern "C" fn voicevox_predict_duration(
    length: usize,
    phoneme_vector: *mut i64,
    speaker_id: u32,
    output_predict_duration_data_length: *mut usize,
    output_predict_duration_data: *mut *mut f32,
) -> VoicevoxResultCode {
    into_result_code_with_error((|| {
        let output_vec = lock_internal().predict_duration(
            std::slice::from_raw_parts_mut(phoneme_vector, length),
            speaker_id,
        )?;
        let (ptr, size) = leak_vec_and_store_length(output_vec);

        output_predict_duration_data_length.write(size);
        output_predict_duration_data.write(ptr);

        Ok(())
    })())
}

/// ::voicevox_predict_durationで出力されたデータを解放する
/// @param[in] predict_duration_data 確保されたメモリ領域
///
/// # Safety
/// @param predict_duration_data 実行後に割り当てられたメモリ領域が解放される
#[no_mangle]
pub unsafe extern "C" fn voicevox_predict_duration_data_free(predict_duration_data: *mut f32) {
    drop(restore_vec(predict_duration_data as *const f32));
}

/// モーラごとのF0を推論する
/// @param [in] length vowel_phoneme_vector, consonant_phoneme_vector, start_accent_vector, end_accent_vector, start_accent_phrase_vector, end_accent_phrase_vector, output のデータ長
/// @param [in] vowel_phoneme_vector 母音の音素データ
/// @param [in] consonant_phoneme_vector 子音の音素データ
/// @param [in] start_accent_vector アクセントの開始位置のデータ
/// @param [in] end_accent_vector アクセントの終了位置のデータ
/// @param [in] start_accent_phrase_vector アクセント句の開始位置のデータ
/// @param [in] end_accent_phrase_vector アクセント句の終了位置のデータ
/// @param [in] speaker_id 話者ID
/// @param [out] output_predict_intonation_data_length 出力データのサイズ
/// @param [out] output_predict_intonation_data データの出力先
/// @return 結果コード #VoicevoxResultCode
///
/// # Safety
/// @param vowel_phoneme_vector 必ずlengthの長さだけデータがある状態で渡すこと
/// @param consonant_phoneme_vector 必ずlengthの長さだけデータがある状態で渡すこと
/// @param start_accent_vector 必ずlengthの長さだけデータがある状態で渡すこと
/// @param end_accent_vector 必ずlengthの長さだけデータがある状態で渡すこと
/// @param start_accent_phrase_vector 必ずlengthの長さだけデータがある状態で渡すこと
/// @param end_accent_phrase_vector 必ずlengthの長さだけデータがある状態で渡すこと
/// @param output_predict_intonation_data_length uintptr_t 分のメモリ領域が割り当てられていること
/// @param output_predict_intonation_data 成功後にメモリ領域が割り当てられるので ::voicevox_predict_intonation_data_free で解放する必要がある
#[no_mangle]
pub unsafe extern "C" fn voicevox_predict_intonation(
    length: usize,
    vowel_phoneme_vector: *mut i64,
    consonant_phoneme_vector: *mut i64,
    start_accent_vector: *mut i64,
    end_accent_vector: *mut i64,
    start_accent_phrase_vector: *mut i64,
    end_accent_phrase_vector: *mut i64,
    speaker_id: u32,
    output_predict_intonation_data_length: *mut usize,
    output_predict_intonation_data: *mut *mut f32,
) -> VoicevoxResultCode {
    into_result_code_with_error((|| {
        let output_vec = lock_internal().predict_intonation(
            length,
            std::slice::from_raw_parts(vowel_phoneme_vector, length),
            std::slice::from_raw_parts(consonant_phoneme_vector, length),
            std::slice::from_raw_parts(start_accent_vector, length),
            std::slice::from_raw_parts(end_accent_vector, length),
            std::slice::from_raw_parts(start_accent_phrase_vector, length),
            std::slice::from_raw_parts(end_accent_phrase_vector, length),
            speaker_id,
        )?;
        let (ptr, len) = leak_vec_and_store_length(output_vec);
        output_predict_intonation_data.write(ptr);
        output_predict_intonation_data_length.write(len);

        Ok(())
    })())
}

/// ::voicevox_predict_intonationで出力されたデータを解放する
/// @param[in] predict_intonation_data 確保されたメモリ領域
///
/// # Safety
/// @param predict_intonation_data 実行後に割り当てられたメモリ領域が解放される
#[no_mangle]
pub unsafe extern "C" fn voicevox_predict_intonation_data_free(predict_intonation_data: *mut f32) {
    drop(restore_vec(predict_intonation_data as *const f32));
}

/// decodeを実行する
/// @param [in] length f0 , output のデータ長及び phoneme のデータ長に関連する
/// @param [in] phoneme_size 音素のサイズ phoneme のデータ長に関連する
/// @param [in] f0 基本周波数
/// @param [in] phoneme_vector 音素データ
/// @param [in] speaker_id 話者ID
/// @param [out] output_decode_data_length 出力先データのサイズ
/// @param [out] output_decode_data データ出力先
/// @return 結果コード #VoicevoxResultCode
///
/// # Safety
/// @param f0 必ず length の長さだけデータがある状態で渡すこと
/// @param phoneme_vector 必ず length * phoneme_size の長さだけデータがある状態で渡すこと
/// @param output_decode_data_length uintptr_t 分のメモリ領域が割り当てられていること
/// @param output_decode_data 成功後にメモリ領域が割り当てられるので ::voicevox_decode_data_free で解放する必要がある
#[no_mangle]
pub unsafe extern "C" fn voicevox_decode(
    length: usize,
    phoneme_size: usize,
    f0: *mut f32,
    phoneme_vector: *mut f32,
    speaker_id: u32,
    output_decode_data_length: *mut usize,
    output_decode_data: *mut *mut f32,
) -> VoicevoxResultCode {
    into_result_code_with_error((|| {
        let output_vec = lock_internal().decode(
            length,
            phoneme_size,
            std::slice::from_raw_parts(f0, length),
            std::slice::from_raw_parts(phoneme_vector, phoneme_size * length),
            speaker_id,
        )?;
        let (ptr, len) = leak_vec_and_store_length(output_vec);
        output_decode_data.write(ptr);
        output_decode_data_length.write(len);
        Ok(())
    })())
}

/// ::voicevox_decodeで出力されたデータを解放する
/// @param[in] decode_data 確保されたメモリ領域
///
/// # Safety
/// @param decode_data 実行後に割り当てられたメモリ領域が解放される
#[no_mangle]
pub unsafe extern "C" fn voicevox_decode_data_free(decode_data: *mut f32) {
    drop(restore_vec(decode_data))
}

/// Audio query のオプション
#[repr(C)]
pub struct VoicevoxAudioQueryOptions {
    /// aquestalk形式のkanaとしてテキストを解釈する
    kana: bool,
}

/// デフォルトの AudioQuery のオプションを生成する
/// @return デフォルト値が設定された AudioQuery オプション
#[no_mangle]
pub extern "C" fn voicevox_make_default_audio_query_options() -> VoicevoxAudioQueryOptions {
    voicevox_core::AudioQueryOptions::default().into()
}

/// AudioQuery を実行する
/// @param [in] text テキスト
/// @param [in] speaker_id 話者ID
/// @param [in] options AudioQueryのオプション
/// @param [out] output_audio_query_json AudioQuery を json でフォーマットしたもの
/// @return 結果コード #VoicevoxResultCode
///
/// # Safety
/// @param text null終端文字列であること
/// @param output_audio_query_json 自動でheapメモリが割り当てられるので ::voicevox_audio_query_json_free で解放する必要がある
#[no_mangle]
pub unsafe extern "C" fn voicevox_audio_query(
    text: *const c_char,
    speaker_id: u32,
    options: VoicevoxAudioQueryOptions,
    output_audio_query_json: *mut *mut c_char,
) -> VoicevoxResultCode {
    into_result_code_with_error((|| {
        let text = CStr::from_ptr(text);
        let audio_query = &create_audio_query(text, speaker_id, Internal::audio_query, options)?;
        write_json_to_ptr(output_audio_query_json, audio_query);
        Ok(())
    })())
}

/// `voicevox_synthesis` のオプション
#[repr(C)]
pub struct VoicevoxSynthesisOptions {
    /// 疑問文の調整を有効にする
    enable_interrogative_upspeak: bool,
}

/// デフォルトの `voicevox_synthesis` のオプションを生成する
/// @return デフォルト値が設定された `voicevox_synthesis` のオプション
#[no_mangle]
pub extern "C" fn voicevox_make_default_synthesis_options() -> VoicevoxSynthesisOptions {
    VoicevoxSynthesisOptions::default()
}

/// AudioQuery から音声合成する
/// @param [in] audio_query_json jsonフォーマットされた AudioQuery
/// @param [in] speaker_id  話者ID
/// @param [in] options AudioQueryから音声合成オプション
/// @param [out] output_wav_length 出力する wav データのサイズ
/// @param [out] output_wav wav データの出力先
/// @return 結果コード #VoicevoxResultCode
///
/// # Safety
/// @param output_wav_length 出力先の領域が確保された状態でpointerに渡されていること
/// @param output_wav 自動で output_wav_length 分のデータが割り当てられるので ::voicevox_wav_free で解放する必要がある
#[no_mangle]
pub unsafe extern "C" fn voicevox_synthesis(
    audio_query_json: *const c_char,
    speaker_id: u32,
    options: VoicevoxSynthesisOptions,
    output_wav_length: *mut usize,
    output_wav: *mut *mut u8,
) -> VoicevoxResultCode {
    into_result_code_with_error((|| {
        let audio_query_json = CStr::from_ptr(audio_query_json)
            .to_str()
            .map_err(|_| CApiError::InvalidUtf8Input)?;
        let audio_query =
            &serde_json::from_str(audio_query_json).map_err(CApiError::InvalidAudioQuery)?;
        let wav = &lock_internal().synthesis(audio_query, speaker_id, options.into())?;
        write_wav_to_ptr(output_wav, output_wav_length, wav);
        Ok(())
    })())
}

/// テキスト音声合成オプション
#[repr(C)]
pub struct VoicevoxTtsOptions {
    /// aquestalk形式のkanaとしてテキストを解釈する
    kana: bool,
    /// 疑問文の調整を有効にする
    enable_interrogative_upspeak: bool,
}

/// デフォルトのテキスト音声合成オプションを生成する
/// @return テキスト音声合成オプション
#[no_mangle]
pub extern "C" fn voicevox_make_default_tts_options() -> VoicevoxTtsOptions {
    voicevox_core::TtsOptions::default().into()
}

/// テキスト音声合成を実行する
/// @param [in] text テキスト
/// @param [in] speaker_id 話者ID
/// @param [in] options テキスト音声合成オプション
/// @param [out] output_wav_length 出力する wav データのサイズ
/// @param [out] output_wav wav データの出力先
/// @return 結果コード #VoicevoxResultCode
///
/// # Safety
/// @param output_wav_length 出力先の領域が確保された状態でpointerに渡されていること
/// @param output_wav は自動で output_wav_length 分のデータが割り当てられるので ::voicevox_wav_free で解放する必要がある
#[no_mangle]
pub unsafe extern "C" fn voicevox_tts(
    text: *const c_char,
    speaker_id: u32,
    options: VoicevoxTtsOptions,
    output_wav_length: *mut usize,
    output_wav: *mut *mut u8,
) -> VoicevoxResultCode {
    into_result_code_with_error((|| {
        let text = ensure_utf8(CStr::from_ptr(text))?;
        let output = lock_internal().tts(text, speaker_id, options.into())?;
        let (ptr, size) = leak_vec_and_store_length(output);
        output_wav.write(ptr);
        output_wav_length.write(size);
        Ok(())
    })())
}

/// jsonフォーマットされた AudioQuery データのメモリを解放する
/// @param [in] audio_query_json 解放する json フォーマットされた AudioQuery データ
///
/// # Safety
/// @param wav 確保したメモリ領域が破棄される
#[no_mangle]
pub unsafe extern "C" fn voicevox_audio_query_json_free(audio_query_json: *mut c_char) {
    libc::free(audio_query_json as *mut c_void);
}

/// wav データのメモリを解放する
/// @param [in] wav 解放する wav データ
///
/// # Safety
/// @param wav 確保したメモリ領域が破棄される
#[no_mangle]
pub unsafe extern "C" fn voicevox_wav_free(wav: *mut u8) {
    drop(restore_vec(wav));
}

/// エラー結果をメッセージに変換する
/// @param [in] result_code メッセージに変換する result_code
/// @return 結果コードを元に変換されたメッセージ文字列
#[no_mangle]
pub extern "C" fn voicevox_error_result_to_message(
    result_code: VoicevoxResultCode,
) -> *const c_char {
    voicevox_core::error_result_to_message::<&'static CStr>(result_code).as_ptr() as *const c_char
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use pretty_assertions::assert_eq;
    use voicevox_core::Error;

    #[rstest]
    #[case(Ok(()), VoicevoxResultCode::VOICEVOX_RESULT_OK)]
    #[case(
        Err(Error::NotLoadedOpenjtalkDict),
        VoicevoxResultCode::VOICEVOX_RESULT_NOT_LOADED_OPENJTALK_DICT_ERROR
    )]
    #[case(
        Err(Error::LoadModel {
            path: "path/to/model.onnx".into(),
            source: anyhow!("some load model error"),
        }),
        VoicevoxResultCode::VOICEVOX_RESULT_LOAD_MODEL_ERROR
    )]
    #[case(
        Err(Error::GetSupportedDevices(anyhow!("some get supported devices error"))),
        VoicevoxResultCode::VOICEVOX_RESULT_GET_SUPPORTED_DEVICES_ERROR
    )]
    fn into_result_code_with_error_works(
        #[case] result: Result<()>,
        #[case] expected: VoicevoxResultCode,
    ) {
        let actual = into_result_code_with_error(result.map_err(Into::into));
        assert_eq!(expected, actual);
    }
}
