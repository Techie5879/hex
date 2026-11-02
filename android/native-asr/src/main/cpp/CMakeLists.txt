cmake_minimum_required(VERSION 3.22)
project(hexasr C CXX)
include(FetchContent)

# The same engine and revision as the desktop crate (Cargo.toml's patched transcribe-cpp).
set(TRANSCRIBE_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(TRANSCRIBE_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(TRANSCRIBE_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(TRANSCRIBE_BUILD_SHARED OFF CACHE BOOL "" FORCE)
set(TRANSCRIBE_USE_OPENMP OFF CACHE BOOL "" FORCE)
set(TRANSCRIBE_USE_SYSTEM_BLAS OFF CACHE BOOL "" FORCE)
set(TRANSCRIBE_INSTALL OFF CACHE BOOL "" FORCE)
set(GGML_OPENMP OFF CACHE BOOL "" FORCE)
set(GGML_NATIVE OFF CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
if(ANDROID_ABI STREQUAL "arm64-v8a")
    # Snapdragon 8+ Gen 1 and most phones since 2022. The engine process isolates a SIGILL elsewhere.
    set(GGML_CPU_ARM_ARCH "armv8.2-a+dotprod+fp16+i8mm" CACHE STRING "" FORCE)
else()
    # The software emulator has SSE4.2 but no AVX.
    set(TRANSCRIBE_X86_CONSERVATIVE ON CACHE BOOL "" FORCE)
    set(GGML_SSE42 ON CACHE BOOL "" FORCE)
endif()

FetchContent_Declare(transcribe
    GIT_REPOSITORY https://github.com/anomalyco/transcribe.cpp
    GIT_TAG 7045d1c0fe7cee37747b2792977421d435e75fca
    GIT_SHALLOW FALSE)
FetchContent_MakeAvailable(transcribe)
# Its build assumes it is the top-level project.
target_include_directories(transcribe PUBLIC ${transcribe_SOURCE_DIR}/include)

add_library(hexasr SHARED hexasr.cpp)
target_link_libraries(hexasr PRIVATE transcribe log)
target_compile_features(hexasr PRIVATE cxx_std_17)
target_link_options(hexasr PRIVATE
    -Wl,--exclude-libs,ALL
    -Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/jni.map
    -Wl,-z,max-page-size=16384)
