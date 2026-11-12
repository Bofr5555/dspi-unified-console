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
    qml/tools/AutoEqWindow.qml
    qml/tools/AutoEqRow.qml
    qml/tools/AutoEqUpdateDialog.qml
    qml/tools/AutoEqRebuild.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeFiles.cpp smoke/SmokeAutoEq.cpp)

# The rebuild's fetch needs Qt Network, and the built-in AutoEQ database (the
# one every client ships, crates/dspi-session/autoeq) is a resource. Both go on
# the targets once they exist, at the end of the directory: the targets are
# made after the work packages' lists are read.
find_package(Qt6 6.8 REQUIRED COMPONENTS Network)
function(dspi_qp15_targets)
    foreach(target DSPiConsole DSPiConsoleSmoke)
        if(TARGET ${target})
            target_link_libraries(${target} PRIVATE Qt6::Network)
            qt_add_resources(${target} dspi_qp15_autoeq
                PREFIX "/autoeq"
                BASE "${DSPI_REPO}/crates/dspi-session/autoeq"
                FILES "${DSPI_REPO}/crates/dspi-session/autoeq/autoeq_database.json")
        endif()
    endforeach()
endfunction()
cmake_language(DEFER CALL dspi_qp15_targets)
