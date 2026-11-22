# -- WP QP15 -- Files and AutoEQ: the File and AutoEQ menus, the unsaved-changes
# guard, the import pickers and reports, the AutoEQ browser and its rebuild
list(APPEND DSPI_APP_SOURCES
    src/controllers/UnsavedGuard.h src/controllers/UnsavedGuard.cpp
    src/controllers/FilesController.h src/controllers/FilesController.cpp
    src/controllers/AutoEqController.h src/controllers/AutoEqController.cpp)
list(APPEND DSPI_QML_FILES
    qml/files/FilesHost.qml
    qml/files/NoticeDialog.qml
    qml/files/ImportFilters.qml
    qml/files/ImportConfig.qml
    qml/files/ImportProgress.qml
    qml/tools/AutoEqPanel.qml   # -- WP LAY1b -- was AutoEqWindow.qml
    qml/tools/AutoEqRow.qml
    qml/tools/AutoEqUpdateDialog.qml
    qml/tools/AutoEqRebuild.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeFiles.cpp smoke/SmokeAutoEq.cpp)

# The rebuild's fetch needs Qt Network, and the built-in AutoEQ database (the
# one every client ships, crates/dspi-session/autoeq) is a resource on each
# target (-- WP QPI2 -- through the base's lists and hook)
list(APPEND DSPI_QT_COMPONENTS Network)
list(APPEND DSPI_APP_LIBS Qt6::Network)
function(dspi_qp15_target target)
    qt_add_resources(${target} dspi_qp15_autoeq
        PREFIX "/autoeq"
        BASE "${DSPI_REPO}/crates/dspi-session/autoeq"
        FILES "${DSPI_REPO}/crates/dspi-session/autoeq/autoeq_database.json")
endfunction()
list(APPEND DSPI_APP_HOOKS dspi_qp15_target)
