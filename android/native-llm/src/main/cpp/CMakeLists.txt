cmake_minimum_required(VERSION 3.22)
project(hexllm C CXX)
include(FetchContent)

# Assert messages embed source paths. Keep the builder's directories out of the shipped library.
add_compile_options(
    -ffile-prefix-map=${CMAKE_SOURCE_DIR}=src
    -ffile-prefix-map=${CMAKE_BINARY_DIR}=build
    -ffile-prefix-map=${ANDROID_NDK}=ndk)

set(LLAMA_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_SERVER OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_COMMON OFF CACHE BOOL "" FORCE)
set(LLAMA_CURL OFF CACHE BOOL "" FORCE)
set(GGML_OPENMP OFF CACHE BOOL "" FORCE)
set(GGML_NATIVE OFF CACHE BOOL "" FORCE)
set(GGML_LLAMAFILE ON CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
if(ANDROID_ABI STREQUAL "arm64-v8a")
    set(GGML_CPU_ARM_ARCH "armv8.2-a+dotprod+fp16+i8mm" CACHE STRING "" FORCE)
else()
    set(GGML_AVX OFF CACHE BOOL "" FORCE)
    set(GGML_AVX2 OFF CACHE BOOL "" FORCE)
    set(GGML_AVX512 OFF CACHE BOOL "" FORCE)
    set(GGML_FMA OFF CACHE BOOL "" FORCE)
    set(GGML_F16C OFF CACHE BOOL "" FORCE)
    set(GGML_BMI2 OFF CACHE BOOL "" FORCE)
    set(GGML_SSE42 ON CACHE BOOL "" FORCE)
endif()

FetchContent_Declare(llama
    GIT_REPOSITORY https://github.com/ggml-org/llama.cpp
    GIT_TAG 8642541b255bfb5dc39cd351a178be09013a55d6
    GIT_SHALLOW FALSE)
FetchContent_MakeAvailable(llama)

add_library(hexllm SHARED hexllm.cpp)
target_link_libraries(hexllm PRIVATE llama log)
target_compile_features(hexllm PRIVATE cxx_std_17)
target_link_options(hexllm PRIVATE
    -Wl,--exclude-libs,ALL
    -Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/jni.map
    -Wl,-z,max-page-size=16384)
