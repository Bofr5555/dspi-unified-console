# -- WP DL0 -- The Modern design language's shared components (DESREV's
# DESIGN-LANGUAGE.md, component map): chips, notices, empty states, list and
# form rows, the graph frame and its axis labels, the content column.
list(APPEND DSPI_QML_FILES
    qml/components/AxisLabels.qml
    qml/components/ChipGroup.qml
    qml/components/ContentColumn.qml
    qml/components/EmptyState.qml
    qml/components/FormRow.qml
    qml/components/GraphFrame.qml
    qml/components/ListRow.qml
    qml/components/NoticeRow.qml
    qml/components/OptionChip.qml
    qml/components/ToggleChip.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeDl0.cpp)
