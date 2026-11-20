/* Minimal replacement for the header CMake's GenerateExportHeader produces.
 * Static linking needs no __declspec, so every export macro is empty. */
#ifndef XEVD_EXPORTS_H
#define XEVD_EXPORTS_H
#define XEVD_EXPORT
#define XEVD_NO_EXPORT
#endif
