# -- WP LAY8 -- Modern's AI button and its popover, over AiController
list(APPEND DSPI_QML_FILES
    qml/modern/AiButton.qml qml/modern/AiPopover.qml
    qml/ai/AiEnableQuestion.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay8.cpp)
