# -- WP LAY3b -- Actions where they act: the menus of MenuTable's places and the
# right-click menus
list(APPEND DSPI_QML_FILES qml/modern/ActionMenu.qml qml/modern/ContextMenus.qml)
list(APPEND DSPI_QML_SINGLETONS qml/modern/ContextMenus.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay3b.cpp)
