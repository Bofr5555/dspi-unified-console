# -- WP SWAPFIX -- A re-read keeps unsaved changes (the questions still ask), one delay line
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeSwapfix.cpp)

# -- WP SWAPFIX -- (QP22b F7) On Windows the smoke driver builds with the
# sections that need no D-Bus, no fake hub and no Linux desktop: the driver,
# the base, the core, the components, the launches and the hardware pass
# (run-hw.sh's sections, as QP22b ran them in the Windows VM). This file comes
# after the work packages' lists (name order), so it has the last word.
if(WIN32)
    set(DSPI_SMOKE_SOURCES
        smoke/SmokeDriver.h smoke/SmokeDriver.cpp smoke/SmokeProbe.h
        smoke/SmokeBase.cpp smoke/SmokeCore.cpp smoke/SmokeComponents.cpp
        smoke/SmokeLaunches.cpp smoke/SmokeHw.cpp)
endif()
