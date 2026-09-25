#include "scene_render/diagnostics.h"

#include <stdarg.h>

void sr_diag_init(SrDiagnostics *diag, const char *source, FILE *stream) {
    *diag = (SrDiagnostics){source ? source : "<scene>", stream ? stream : stderr,
                            0, 0, false};
}

static void sr_diag_message(SrDiagnostics *diag, const char *level, size_t line,
                            const char *element, const char *attribute,
                            const char *format, va_list args) {
    fprintf(diag->stream, "%s", diag->source ? diag->source : "<scene>");
    if (line) {
        fprintf(diag->stream, ":%zu", line);
    }
    fprintf(diag->stream, ": %s", level);
    if (element) {
        fprintf(diag->stream, ": <%s>", element);
    }
    if (attribute) {
        fprintf(diag->stream, " @%s", attribute);
    }
    fprintf(diag->stream, ": ");
    vfprintf(diag->stream, format, args);
    fputc('\n', diag->stream);
}

void sr_diag_error(SrDiagnostics *diag, size_t line, const char *element,
                   const char *attribute, const char *format, ...) {
    ++diag->errors;
    va_list args;
    va_start(args, format);
    sr_diag_message(diag, "error", line, element, attribute, format, args);
    va_end(args);
}

void sr_diag_warning(SrDiagnostics *diag, size_t line, const char *element,
                     const char *attribute, const char *format, ...) {
    ++diag->warnings;
    va_list args;
    va_start(args, format);
    sr_diag_message(diag, "warning", line, element, attribute, format, args);
    va_end(args);
}

void sr_diag_info(SrDiagnostics *diag, const char *format, ...) {
    if (!diag->verbose) {
        return;
    }
    fprintf(diag->stream, "info: ");
    va_list args;
    va_start(args, format);
    vfprintf(diag->stream, format, args);
    va_end(args);
    fputc('\n', diag->stream);
}
