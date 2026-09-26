cmake_minimum_required(VERSION 3.20)
project(qwen3asr_native_worker LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT AUDIOCPP_SOURCE_DIR OR NOT AUDIOCPP_BINARY_DIR)
    message(FATAL_ERROR "Set AUDIOCPP_SOURCE_DIR and AUDIOCPP_BINARY_DIR to the pinned audio.cpp source and build")
endif()
if(NOT EXISTS "${AUDIOCPP_SOURCE_DIR}/include/audiocpp.h")
    message(FATAL_ERROR "audio.cpp C API header is missing")
endif()

add_executable(qwen3asr-worker worker.cpp "${AUDIOCPP_SOURCE_DIR}/external/cJSON/cJSON.c")
target_include_directories(qwen3asr-worker PRIVATE
    "${AUDIOCPP_SOURCE_DIR}/include"
    "${AUDIOCPP_SOURCE_DIR}/external/cJSON")
target_compile_definitions(qwen3asr-worker PRIVATE
    AUDIOCPP_USE_DLL
    QWEN_AUDIOCPP_COMMIT="${QWEN_AUDIOCPP_COMMIT}")
if(NATIVE_CUDA_COMPILED)
    if(NOT DEFINED NATIVE_CUDA_MIN_CC)
        set(NATIVE_CUDA_MIN_CC 86)
    endif()
    if(NOT NATIVE_CUDA_MIN_CC MATCHES "^[0-9]+$")
        message(FATAL_ERROR "Set NATIVE_CUDA_MIN_CC to the lowest compiled CUDA architecture")
    endif()
    target_compile_definitions(qwen3asr-worker PRIVATE
        NATIVE_CUDA_COMPILED=1 NATIVE_CUDA_MIN_CC=${NATIVE_CUDA_MIN_CC})
endif()
if(MSVC)
    target_compile_options(qwen3asr-worker PRIVATE /utf-8 /EHsc)
    target_link_libraries(qwen3asr-worker PRIVATE psapi)
    set(AUDIOCPP_IMPORT "${AUDIOCPP_BINARY_DIR}/audiocpp.lib")
    set(AUDIOCPP_SHARED "${AUDIOCPP_BINARY_DIR}/bin/audiocpp.dll")
else()
    set(AUDIOCPP_IMPORT "${AUDIOCPP_BINARY_DIR}/bin/libaudiocpp.so")
    set(AUDIOCPP_SHARED "${AUDIOCPP_IMPORT}")
    if(APPLE)
        set(AUDIOCPP_IMPORT "${AUDIOCPP_BINARY_DIR}/bin/libaudiocpp.dylib")
        set(AUDIOCPP_SHARED "${AUDIOCPP_IMPORT}")
    endif()
    target_link_libraries(qwen3asr-worker PRIVATE ${CMAKE_DL_LIBS})
endif()
if(NOT EXISTS "${AUDIOCPP_IMPORT}")
    message(FATAL_ERROR "Build audio.cpp target audiocpp first: ${AUDIOCPP_IMPORT}")
endif()
target_link_libraries(qwen3asr-worker PRIVATE "${AUDIOCPP_IMPORT}")
set_target_properties(qwen3asr-worker PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin")
add_custom_command(TARGET qwen3asr-worker POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${AUDIOCPP_SHARED}" "$<TARGET_FILE_DIR:qwen3asr-worker>")
