# -- WP QP5 -- Dashboard (the Overview): cards, layout popup, meters, stale state
list(APPEND DSPI_APP_SOURCES
    src/controllers/DashboardController.h src/controllers/DashboardController.cpp
    src/render/Meter.h src/render/Meter.cpp)
list(APPEND DSPI_QML_FILES
    qml/main/Dashboard.qml qml/main/DashboardCard.qml qml/main/DashboardLayoutPopup.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeDashboard.cpp)
# The QML type registrations include the headers by bare name, and the base
# does not list src/render among the include directories yet
include_directories("${CMAKE_CURRENT_SOURCE_DIR}/src/render")
