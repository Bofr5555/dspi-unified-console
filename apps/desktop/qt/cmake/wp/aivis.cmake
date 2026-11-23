# -- WP AIVIS -- The screen shows what an AI agent does: the working strip in both
# layouts, the mark on a control an agent changed
list(APPEND DSPI_QML_FILES
    qml/ai/AiWorkBanner.qml qml/ai/AiWorkMark.qml qml/ai/AiTouched.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeAivis.cpp)
