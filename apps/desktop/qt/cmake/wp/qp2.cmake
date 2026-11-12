# -- WP QP2 -- Filter lists and channel headers
list(APPEND DSPI_APP_SOURCES
    src/controllers/ChannelController.h src/controllers/ChannelController.cpp)
list(APPEND DSPI_QML_FILES
    qml/main/ChannelValueField.qml
    qml/main/BandRow.qml
    qml/main/FilterList.qml
    qml/main/Linkwitz.qml
    qml/main/InputDetail.qml
    qml/main/OutputCard.qml
    qml/main/OutputDetail.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeChannel.cpp)
list(APPEND DSPI_SMOKE_QML_FILES smoke/SmokeChannelWindow.qml)
