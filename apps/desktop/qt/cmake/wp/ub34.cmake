# -- WP UB34 -- A/B as saved against now in the top bar (UXP B3), an All
# devices link in the dock (UXP B4): the smoke sections
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeUb34.cpp)
list(APPEND DSPI_APP_SOURCES
    src/controllers/DeviceLinkController.h src/controllers/DeviceLinkController.cpp)
