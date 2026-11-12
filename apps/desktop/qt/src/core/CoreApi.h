#ifndef COREAPI_H
#define COREAPI_H

// The core's C ABI. cbindgen writes the header without C++ guards, so it is
// included through this file everywhere in the app.
extern "C" {
#include "dspi_core.h"
}

#endif // COREAPI_H
