#include "audiocpp.h"
#include "cJSON.h"

#include <algorithm>
#include <cctype>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#include <psapi.h>
#else
#include <dlfcn.h>
#include <sys/resource.h>
#endif

#ifndef QWEN_AUDIOCPP_COMMIT
#define QWEN_AUDIOCPP_COMMIT "unknown"
#endif
#ifndef NATIVE_CUDA_COMPILED
#define NATIVE_CUDA_COMPILED 0
#endif
#ifndef NATIVE_CUDA_MIN_CC
#define NATIVE_CUDA_MIN_CC 0
#endif

namespace {

using Json = std::unique_ptr<cJSON, decltype(&cJSON_Delete)>;
using Registry = std::unique_ptr<audiocpp_registry, decltype(&audiocpp_registry_free)>;
using Model = std::unique_ptr<audiocpp_model, decltype(&audiocpp_model_free)>;
using Session = std::unique_ptr<audiocpp_session, decltype(&audiocpp_session_free)>;
using Request = std::unique_ptr<audiocpp_request, decltype(&audiocpp_request_free)>;
using Result = std::unique_ptr<audiocpp_result, decltype(&audiocpp_result_free)>;

void checked(audiocpp_status status) {
    if (status != AUDIOCPP_OK) {
        const std::string detail = audiocpp_last_error();
        throw std::runtime_error(std::string(audiocpp_status_string(status)) + ": " + detail);
    }
}

Json object() { return Json(cJSON_CreateObject(), cJSON_Delete); }

void emit(const cJSON *value) {
    char *bytes = cJSON_PrintUnformatted(value);
    if (!bytes) throw std::runtime_error("JSON serialization failed");
    std::cout << bytes << '\n' << std::flush;
    cJSON_free(bytes);
}

int fatal(const std::exception &error) {
    auto reply = object();
    cJSON_AddStringToObject(reply.get(), "error", error.what());
    emit(reply.get());
    std::cerr << error.what() << '\n';
    return 1;
}

bool read_json_line(std::string &line) {
    line.clear();
    for (;;) {
        const int ch = std::cin.get();
        if (ch == '\n') return true;
        if (ch == std::char_traits<char>::eof()) return !line.empty();
        if (line.size() >= 1024 * 1024) throw std::runtime_error("Request exceeds 1 MiB");
        line.push_back(static_cast<char>(ch));
    }
}

std::string utf8(const std::wstring &wide) {
#ifdef _WIN32
    if (wide.empty()) return {};
    const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide.c_str(),
                                         static_cast<int>(wide.size()), nullptr, 0, nullptr, nullptr);
    if (size <= 0) throw std::runtime_error("Invalid UTF-16 command argument");
    std::string result(static_cast<size_t>(size), '\0');
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, wide.c_str(),
                        static_cast<int>(wide.size()), result.data(), size, nullptr, nullptr);
    return result;
#else
    return std::string(wide.begin(), wide.end());
#endif
}

std::filesystem::path native_path(const std::string &path) {
    return std::filesystem::u8path(path);
}

uint16_t le16(const unsigned char *p) {
    return static_cast<uint16_t>(p[0] | (static_cast<uint16_t>(p[1]) << 8));
}
uint32_t le32(const unsigned char *p) {
    return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8) |
           (static_cast<uint32_t>(p[2]) << 16) | (static_cast<uint32_t>(p[3]) << 24);
}

// Rust supplies one bounded, decoded 16 kHz PCM16 mono chunk per request.
std::vector<float> read_wav(const std::string &path) {
    std::ifstream stream(native_path(path), std::ios::binary);
    if (!stream) throw std::runtime_error("Cannot open WAV: " + path);
    unsigned char riff[12]{};
    stream.read(reinterpret_cast<char *>(riff), sizeof(riff));
    if (!stream || std::memcmp(riff, "RIFF", 4) || std::memcmp(riff + 8, "WAVE", 4)) {
        throw std::runtime_error("Expected RIFF/WAVE input");
    }
    bool valid_fmt = false;
    std::vector<float> samples;
    while (stream) {
        unsigned char header[8]{};
        stream.read(reinterpret_cast<char *>(header), sizeof(header));
        if (!stream) break;
        const uint32_t length = le32(header + 4);
        if (!std::memcmp(header, "fmt ", 4)) {
            if (length < 16 || length > 1024) throw std::runtime_error("Invalid WAV fmt chunk");
            std::vector<unsigned char> fmt(length);
            stream.read(reinterpret_cast<char *>(fmt.data()), length);
            if (!stream) throw std::runtime_error("Truncated WAV fmt chunk");
            valid_fmt = le16(fmt.data()) == 1 && le16(fmt.data() + 2) == 1 &&
                        le32(fmt.data() + 4) == 16000 && le16(fmt.data() + 14) == 16;
            if (!valid_fmt) throw std::runtime_error("WAV must be 16 kHz mono PCM16");
        } else if (!std::memcmp(header, "data", 4)) {
            if (!valid_fmt) throw std::runtime_error("WAV fmt must precede data");
            if (length == 0 || length % 2 || length > 60u * 16000u * 2u) {
                throw std::runtime_error("WAV chunk must contain at most 60 seconds of PCM16");
            }
            std::vector<unsigned char> pcm(length);
            stream.read(reinterpret_cast<char *>(pcm.data()), length);
            if (!stream) throw std::runtime_error("Truncated WAV data chunk");
            samples.reserve(length / 2);
            for (size_t i = 0; i < length; i += 2) {
                samples.push_back(static_cast<float>(static_cast<int16_t>(le16(pcm.data() + i))) / 32768.0f);
            }
            return samples;
        } else {
            stream.seekg(static_cast<std::streamoff>(length), std::ios::cur);
        }
        if (length & 1) stream.seekg(1, std::ios::cur);
    }
    throw std::runtime_error("WAV data chunk is missing");
}

uint64_t peak_rss_bytes() {
#ifdef _WIN32
    PROCESS_MEMORY_COUNTERS info{};
    info.cb = sizeof(info);
    return GetProcessMemoryInfo(GetCurrentProcess(), &info, sizeof(info)) ?
        static_cast<uint64_t>(info.PeakWorkingSetSize) : 0;
#else
    struct rusage info{};
    if (getrusage(RUSAGE_SELF, &info) != 0) return 0;
#ifdef __APPLE__
    return static_cast<uint64_t>(info.ru_maxrss);
#else
    return static_cast<uint64_t>(info.ru_maxrss) * 1024;
#endif
#endif
}

struct CudaCapability {
    bool driver_visible = false;
    int compute_capability = 0;
};

CudaCapability cuda_capability() {
    CudaCapability result;
    if (!NATIVE_CUDA_COMPILED) return result;
#ifdef _WIN32
    HMODULE driver = LoadLibraryW(L"nvcuda.dll");
    if (!driver) return result;
    using Init = int (*)(unsigned int);
    using Count = int (*)(int *);
    using Device = int (*)(int *, int);
    using Capability = int (*)(int *, int *, int);
    const auto init = reinterpret_cast<Init>(GetProcAddress(driver, "cuInit"));
    const auto count = reinterpret_cast<Count>(GetProcAddress(driver, "cuDeviceGetCount"));
    const auto get_device = reinterpret_cast<Device>(GetProcAddress(driver, "cuDeviceGet"));
    const auto get_capability = reinterpret_cast<Capability>(GetProcAddress(driver, "cuDeviceComputeCapability"));
    int devices = 0;
    result.driver_visible = init && count && init(0) == 0 && count(&devices) == 0 && devices > 0;
    int device = 0, major = 0, minor = 0;
    if (result.driver_visible && get_device && get_capability && get_device(&device, 0) == 0 &&
        get_capability(&major, &minor, device) == 0) {
        result.compute_capability = major * 10 + minor;
    }
    FreeLibrary(driver);
    return result;
#else
    void *driver = dlopen("libcuda.so.1", RTLD_LAZY);
    if (!driver) return result;
    using Init = int (*)(unsigned int);
    using Count = int (*)(int *);
    using Device = int (*)(int *, int);
    using Capability = int (*)(int *, int *, int);
    const auto init = reinterpret_cast<Init>(dlsym(driver, "cuInit"));
    const auto count = reinterpret_cast<Count>(dlsym(driver, "cuDeviceGetCount"));
    const auto get_device = reinterpret_cast<Device>(dlsym(driver, "cuDeviceGet"));
    const auto get_capability = reinterpret_cast<Capability>(dlsym(driver, "cuDeviceComputeCapability"));
    int devices = 0;
    result.driver_visible = init && count && init(0) == 0 && count(&devices) == 0 && devices > 0;
    int device = 0, major = 0, minor = 0;
    if (result.driver_visible && get_device && get_capability && get_device(&device, 0) == 0 &&
        get_capability(&major, &minor, device) == 0) {
        result.compute_capability = major * 10 + minor;
    }
    dlclose(driver);
    return result;
#endif
}

std::string string_field(const cJSON *root, const char *key, const std::string &fallback = {}) {
    const cJSON *value = cJSON_GetObjectItemCaseSensitive(root, key);
    if (!value) return fallback;
    if (!cJSON_IsString(value)) throw std::runtime_error(std::string("Expected string field: ") + key);
    return value->valuestring;
}

std::string asr_language(const std::string &language) {
    std::string normalized = language;
    std::transform(normalized.begin(), normalized.end(), normalized.begin(),
                   [](unsigned char ch) { return static_cast<char>(std::tolower(ch)); });
    return normalized.empty() || normalized == "auto" ? "Auto" : language;
}

void add_metrics(cJSON *reply) {
    cJSON *metrics = cJSON_AddObjectToObject(reply, "metrics");
    cJSON_AddNumberToObject(metrics, "peak_process_rss_bytes", static_cast<double>(peak_rss_bytes()));
}

struct Options {
    std::string model;
    std::string task;
    std::string backend = "cpu";
    std::string language;
    std::string prompt;
    std::string model_spec;
    int threads = 4;
    int max_tokens = 512;
    bool probe = false;
};

Options parse_arguments(const std::vector<std::string> &args) {
    Options opts;
    for (size_t i = 1; i < args.size(); ++i) {
        const std::string &arg = args[i];
        if (arg == "--probe") { opts.probe = true; continue; }
        if (i + 1 >= args.size()) throw std::runtime_error("Missing value for " + arg);
        const std::string value = args[++i];
        if (arg == "--model") opts.model = value;
        else if (arg == "--task") opts.task = value;
        else if (arg == "--backend") opts.backend = value;
        else if (arg == "--threads") opts.threads = std::stoi(value);
        else if (arg == "--max-tokens") opts.max_tokens = std::stoi(value);
        else if (arg == "--language") opts.language = value;
        else if (arg == "--prompt") opts.prompt = value;
        else if (arg == "--model-spec-override") opts.model_spec = value;
        else throw std::runtime_error("Unknown argument: " + arg);
    }
    if (opts.probe) return opts;
    if (opts.model.empty() || (opts.task != "asr" && opts.task != "align") ||
        (opts.backend != "cpu" && opts.backend != "cuda") || opts.threads < 1 ||
        opts.threads > 256 || opts.max_tokens < 1 || opts.max_tokens > 8192) {
        throw std::runtime_error("Require --model, --task asr|align, --backend cpu|cuda, valid threads/max-tokens");
    }
    return opts;
}

void probe() {
    auto reply = object();
    cJSON_AddBoolToObject(reply.get(), "ok", true);
    cJSON_AddStringToObject(reply.get(), "engine_version", audiocpp_build_version());
    cJSON_AddStringToObject(reply.get(), "source_commit", QWEN_AUDIOCPP_COMMIT);
    cJSON_AddNumberToObject(reply.get(), "abi_version", audiocpp_abi_version());
    cJSON_AddBoolToObject(reply.get(), "cpu_available", true);
    cJSON_AddBoolToObject(reply.get(), "cuda_compiled", NATIVE_CUDA_COMPILED);
    const CudaCapability cuda = cuda_capability();
    cJSON_AddBoolToObject(reply.get(), "cuda_driver_visible", cuda.driver_visible);
    cJSON_AddNumberToObject(reply.get(), "cuda_device_compute_capability", cuda.compute_capability);
    cJSON_AddNumberToObject(reply.get(), "cuda_min_compute_capability", NATIVE_CUDA_MIN_CC);
    cJSON_AddBoolToObject(reply.get(), "cuda_available",
                          NATIVE_CUDA_COMPILED && cuda.compute_capability >= NATIVE_CUDA_MIN_CC);
    emit(reply.get());
}

int run(const Options &opts) {
    if (opts.probe) { probe(); return 0; }
    if ((audiocpp_abi_version() >> 16) != AUDIOCPP_ABI_VERSION_MAJOR) {
        throw std::runtime_error("audio.cpp C ABI major version mismatch");
    }
    audiocpp_registry *registry_raw = nullptr;
    checked(audiocpp_registry_create(nullptr, &registry_raw));
    Registry registry(registry_raw, audiocpp_registry_free);
    audiocpp_model_config config{};
    config.family_hint = opts.task == "asr" ? "qwen3_asr" : "qwen3_forced_aligner";
    config.model_spec_override = opts.model_spec.empty() ? nullptr : opts.model_spec.c_str();
    audiocpp_model *model_raw = nullptr;
    checked(audiocpp_model_load(registry.get(), opts.model.c_str(), &config, nullptr, &model_raw));
    Model model(model_raw, audiocpp_model_free);
    const audiocpp_backend_config backend{opts.backend.c_str(), 0, opts.threads};
    audiocpp_session *session_raw = nullptr;
    checked(audiocpp_session_create(model.get(), opts.task.c_str(), "offline", &backend, nullptr, &session_raw));
    Session session(session_raw, audiocpp_session_free);
    auto ready = object();
    cJSON_AddBoolToObject(ready.get(), "ready", true);
    cJSON_AddStringToObject(ready.get(), "backend", opts.backend.c_str());
    cJSON_AddStringToObject(ready.get(), "engine_version", audiocpp_build_version());
    add_metrics(ready.get());
    emit(ready.get());

    std::string line;
    while (read_json_line(line)) {
        Json input(cJSON_Parse(line.c_str()), cJSON_Delete);
        auto reply = object();
        const cJSON *id = input ? cJSON_GetObjectItemCaseSensitive(input.get(), "id") : nullptr;
        if (id) cJSON_AddItemToObject(reply.get(), "id", cJSON_Duplicate(id, true));
        else cJSON_AddNullToObject(reply.get(), "id");
        try {
            if (!input || !cJSON_IsObject(input.get())) throw std::runtime_error("Invalid request JSON object");
            const std::string audio_path = string_field(input.get(), "audio");
            const std::string text = string_field(input.get(), "text", opts.prompt);
            const std::string language = string_field(input.get(), "language", opts.language);
            if (audio_path.empty()) throw std::runtime_error("audio path is required");
            if (opts.task == "align" && (text.empty() || language.empty())) {
                throw std::runtime_error("Alignment requires exact text and language");
            }
            const auto samples = read_wav(audio_path);
            Request request(audiocpp_request_create(), audiocpp_request_free);
            if (!request) throw std::runtime_error("Cannot allocate request");
            checked(audiocpp_request_set_audio(request.get(), samples.data(), samples.size(), 16000, 1));
            if (opts.task == "align") {
                checked(audiocpp_request_set_text(request.get(), text.c_str(), nullptr));
                checked(audiocpp_request_set_text_language(request.get(), language.c_str()));
            } else {
                const std::string model_language = asr_language(language);
                const std::string model_prompt = text.empty() ? opts.prompt : text;
                checked(audiocpp_request_set_text(request.get(), model_prompt.c_str(), model_language.c_str()));
                checked(audiocpp_request_set_option(request.get(), "max_tokens", std::to_string(opts.max_tokens).c_str()));
                checked(audiocpp_request_set_option(request.get(), "audio_chunk_mode", "none"));
            }
            audiocpp_result *result_raw = nullptr;
            checked(audiocpp_session_run(session.get(), request.get(), &result_raw));
            Result result(result_raw, audiocpp_result_free);
            const char *result_text = "";
            const char *result_language = "";
            const auto text_status = audiocpp_result_text(result.get(), &result_text, &result_language);
            if (text_status != AUDIOCPP_OK && text_status != AUDIOCPP_ERR_NOT_AVAILABLE) checked(text_status);
            cJSON_AddStringToObject(reply.get(), "text", result_text);
            const std::string fallback_language =
                opts.task == "asr" && asr_language(language) == "Auto" ? "" : language;
            cJSON_AddStringToObject(reply.get(), "language",
                                    result_language && *result_language ? result_language : fallback_language.c_str());
            cJSON_AddBoolToObject(reply.get(), "truncated", false);
            cJSON *words = cJSON_AddArrayToObject(reply.get(), "words");
            const size_t count = audiocpp_result_word_count(result.get());
            for (size_t i = 0; i < count; ++i) {
                const char *word_text = "";
                int64_t start = 0, end = 0;
                float confidence = 0;
                checked(audiocpp_result_word(result.get(), i, &word_text, &start, &end, &confidence));
                cJSON *word = cJSON_CreateObject();
                cJSON_AddStringToObject(word, "text", word_text);
                cJSON_AddNumberToObject(word, "start", static_cast<double>(start) / 16000.0);
                cJSON_AddNumberToObject(word, "end", static_cast<double>(end) / 16000.0);
                cJSON_AddItemToArray(words, word);
            }
        } catch (const std::exception &error) {
            cJSON_DeleteItemFromObjectCaseSensitive(reply.get(), "text");
            cJSON_DeleteItemFromObjectCaseSensitive(reply.get(), "words");
            cJSON_AddStringToObject(reply.get(), "error", error.what());
        }
        add_metrics(reply.get());
        emit(reply.get());
    }
    return 0;
}

int worker_main(const std::vector<std::string> &args) {
    try { return run(parse_arguments(args)); }
    catch (const std::exception &error) { return fatal(error); }
}

} // namespace

#ifdef _WIN32
int wmain(int argc, wchar_t **argv) {
    try {
        std::vector<std::string> args;
        for (int i = 0; i < argc; ++i) args.push_back(utf8(argv[i]));
        return worker_main(args);
    } catch (const std::exception &error) { return fatal(error); }
}
#else
int main(int argc, char **argv) {
    return worker_main(std::vector<std::string>(argv, argv + argc));
}
#endif
