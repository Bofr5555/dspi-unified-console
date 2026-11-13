# -- WP QP1 -- Sidebar and global controls
list(APPEND DSPI_APP_SOURCES
    src/controllers/SidebarController.h src/controllers/SidebarController.cpp)
list(APPEND DSPI_QML_FILES
    qml/main/ChannelRow.qml qml/main/QuickStrip.qml qml/main/GlobalControls.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeSidebar.cpp)
