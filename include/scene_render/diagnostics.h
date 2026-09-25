#ifndef SCENE_RENDER_DIAGNOSTICS_H
#define SCENE_RENDER_DIAGNOSTICS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>

typedef struct {
    const char *source;
    FILE *stream;
    size_t errors;
    size_t warnings;
    bool verbose;
} SrDiagnostics;

void sr_diag_init(SrDiagnostics *diag, const char *source, FILE *stream);
void sr_diag_error(SrDiagnostics *diag, size_t line, const char *element,
                   const char *attribute, const char *format, ...);
void sr_diag_warning(SrDiagnostics *diag, size_t line, const char *element,
                     const char *attribute, const char *format, ...);
void sr_diag_info(SrDiagnostics *diag, const char *format, ...);

#endif
