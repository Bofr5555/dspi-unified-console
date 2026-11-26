// -- WP DL0 -- The Modern design language's shared layer: the tokens that are
// gated on the layout (no outlines in Modern, Classic's kept), the new
// components (chips, notices, empty states, list and form rows, the graph
// frame) made in a probe window and used as a page would, and the response
// graph's axis labels whole inside the plot.
#include "SmokeDriver.h"

#include "Registrar.h"
#include "LayoutController.h"

#include <QAccessible>
#include <QFile>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQmlContext>
#include <QQmlProperty>
#include <QQuickItem>
#include <QQuickWindow>

#include <functional>
#include <memory>

namespace {

const char kProbe[] = R"QML(
import QtQuick
import QtQuick.Layouts
import UConsole

Window {
    id: probe
    width: 900
    height: 900
    visible: true
    color: Theme.window
    title: "dl0"
    property var toggles: []
    property int alls: 0
    property int chosenIndex: -1
    property int actions: 0
    property int rowClicks: 0
    property int deletes: 0
    property var selected: [0, 2]

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingSection
        ToolHeader {
            objectName: "Dl0.header"
            Layout.fillWidth: true
            iconName: "tube"
            title: "Intro row"
            subtitle: "What the tool does, in one line"
            on: true
        }
        RowLayout {
            spacing: Theme.spacingSection
            Layout.fillWidth: true
            ContentColumn {
                objectName: "Dl0.column"
                spacing: Theme.spacing
                ChipGroup {
                    objectName: "Dl0.chips"
                    Layout.fillWidth: true
                    label: "Outputs"
                    model: ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
                    selected: probe.selected
                    onToggled: (index, on) => probe.toggles = probe.toggles.concat([index + (on ? 100 : 0)])
                    onSetAll: on => probe.alls += on ? 1 : 10
                }
                ChipGroup {
                    objectName: "Dl0.one"
                    exclusive: true
                    label: "Detector"
                    model: [{ text: "Peak", toolTip: "Peak detector" }, { text: "RMS" }]
                    current: probe.chosenIndex < 0 ? 0 : probe.chosenIndex
                    onChosen: index => probe.chosenIndex = index
                }
                RowLayout {
                    spacing: 6
                    ToggleChip { objectName: "Dl0.toggle"; text: "RTA"; iconName: "audio-lines" }
                    ToolSegments {
                        objectName: "Dl0.segments"
                        Layout.fillWidth: true
                        label: "Display"
                        model: ["Curves", "Bars", "Both"]
                        current: 1
                    }
                }
                NoticeRow {
                    objectName: "Dl0.notice"
                    Layout.fillWidth: true
                    kind: "warning"
                    text: "Choose a microphone to measure."
                    actionText: "Choose…"
                    onAction: probe.actions += 1
                }
                FormRow {
                    objectName: "Dl0.form"
                    title: "Attack"
                    caption: "How fast the level comes down"
                    NumericField { objectName: "Dl0.field"; implicitWidth: 88; unit: "ms"; value: 10 }
                }
                FormRow {
                    title: "Mode"
                    AppComboBox { model: ["Smooth", "Fast"] }
                }
                Card {
                    objectName: "Dl0.card"
                    Layout.fillWidth: true
                    title: "Card"
                    Repeater {
                        model: 3
                        ListRow {
                            required property int index
                            objectName: "Dl0.row." + index
                            Layout.fillWidth: true
                            rowIndex: index
                            title: "Preset " + (index + 1)
                            value: (-index) + ".0 dB"
                            selected: index === 1
                            onClicked: probe.rowClicks += 1
                            IconButton { objectName: "Dl0.row." + index + ".delete"; iconName: "trash"; toolTip: "Delete"; onClicked: probe.deletes += 1 }
                        }
                    }
                    // -- WP DL0 fold -- a leading slot whose item is hidden
                    ListRow {
                        objectName: "Dl0.leadRow"
                        Layout.fillWidth: true
                        title: "Lead"
                        leading: Item { visible: false; implicitWidth: 120; implicitHeight: 10 }
                    }
                }
                RowLayout {
                    spacing: Theme.spacingSmall
                    AppButton { objectName: "Dl0.button"; text: "Secondary" }
                    AppButton { text: "Save to Device"; primary: true }
                    BorderedField { objectName: "Dl0.text"; text: "Text" }
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignTop
                spacing: Theme.spacing
                GraphFrame {
                    objectName: "Dl0.graph"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 160
                    xTicks: [{ at: 0, text: "20" }, { at: 0.5, text: "1k" }, { at: 1, text: "20k" }]
                    yTicks: [{ at: 0, text: "+25" }, { at: 0.5, text: "0" }, { at: 1, text: "-25" }]
                    hint: "A hint under the graph."
                    tools: [ ToggleChip { overlay: true; text: "Phase" } ]
                    Rectangle { anchors.centerIn: parent; width: parent.width / 2; height: 2; color: Theme.accent }
                }
                GraphFrame {
                    objectName: "Dl0.emptyGraph"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 100
                    empty: true
                    emptyText: "Disabled"
                }
                // -- WP DL0 fold --
                StatusChip {
                    objectName: "Dl0.status"
                    kind: "warning"
                    text: "Meters stale"
                    toolTip: "No reading for a while."
                }
                EmptyState {
                    objectName: "Dl0.emptyDefault"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 90
                    title: "Default icon"
                }
                EmptyState {
                    objectName: "Dl0.empty"
                    Layout.fillWidth: true
                    Layout.preferredHeight: 200
                    iconName: "plug"
                    title: "No device"
                    text: "Connect one over USB or open the demo."
                    actionText: "Choose a device…"
                    actionPrimary: true
                    onAction: probe.actions += 10
                }
            }
        }
        Item { Layout.fillHeight: true }
    }
}
)QML";

QQuickItem *named(QQuickItem *root, const QString &name) {
    if (!root) return nullptr;
    if (root->objectName() == name) return root;
    for (QQuickItem *c : root->childItems())
        if (QQuickItem *f = named(c, name)) return f;
    return nullptr;
}

QVariant attached(QObject *o, const char *name) {
    return QQmlProperty(o, QString::fromLatin1(name), qmlContext(o)).read();
}

QObject *background(QObject *control) {
    return control ? control->property("background").value<QObject *>() : nullptr;
}

int borderWidth(QObject *rect) {
    QObject *b = rect ? rect->property("border").value<QObject *>() : nullptr;
    return b ? b->property("width").toInt() : -1;
}

// Every text under `root` lies wholly inside it
bool insideOf(QQuickItem *root, int *texts) {
    bool ok = true;
    const QRectF bounds(0, 0, root->width(), root->height());
    for (QQuickItem *c : root->childItems()) {
        if (!c->isVisible() || !c->property("text").isValid()) continue;
        ++*texts;
        const QRectF r = c->mapRectToItem(root, QRectF(0, 0, c->width(), c->height()));
        if (!bounds.contains(r)) {
            std::printf("smoke: dl0: '%s' at %.0f,%.0f %.0fx%.0f outside %.0fx%.0f\n", qPrintable(c->property("text").toString()),
                        r.x(), r.y(), r.width(), r.height(), bounds.width(), bounds.height());
            ok = false;
        }
    }
    return ok;
}

void probeChecks(SmokeDriver &d, QQuickWindow *w, QObject *theme) {
    QQuickItem *root = w->contentItem();
    auto item = [&](const char *name) { return named(root, QString::fromLatin1(name)); };

    d.check("dl0: Modern has no outlines (Theme.lineWidth 0)", theme->property("lineWidth").toInt() == 0);
    d.check("dl0: Modern's table rows are 28 high", theme->property("rowHeight").toInt() == 28);
    d.check("dl0: a card has no outline", borderWidth(item("Dl0.card")) == 0);
    d.check("dl0: a button has no outline", borderWidth(background(item("Dl0.button"))) == 0);
    d.check("dl0: a text field has no outline", borderWidth(background(item("Dl0.text"))) == 0);

    // The chips: state from the owner, clicks and All / None as signals
    QQuickItem *c0 = item("Dl0.chips.chip.0");
    QQuickItem *c1 = item("Dl0.chips.chip.1");
    d.check("dl0: the chip group makes its chips", c0 && c1 && item("Dl0.chips.chip.8"));
    if (c0 && c1) {
        d.check("dl0: the chosen chips follow `selected`", c0->property("checked").toBool() && !c1->property("checked").toBool());
        d.check("dl0: a chosen chip is the selection tone with accent text, not an accent fill", background(c0)
                && background(c0)->property("color").value<QColor>() == theme->property("selection").value<QColor>()
                && c0->property("ink").value<QColor>() == theme->property("accentText").value<QColor>());
        d.check("dl0: a chip has its accessible role and name",
                attached(c1, "Accessible.role").toInt() == QAccessible::CheckBox && attached(c1, "Accessible.name").toString() == QLatin1String("2"));
        QMetaObject::invokeMethod(c1, "clicked");
        d.spin(30);
        d.check("dl0: a click asks the owner (toggled(1, on))", w->property("toggles").toList() == QVariantList{ 101 });
        d.check("dl0: and the chip waits for the owner", !c1->property("checked").toBool());
        w->setProperty("selected", QVariantList{ 0, 1, 2 });
        d.spin(30);
        d.check("dl0: the owner's write shows", c1->property("checked").toBool());
        QQuickItem *all = item("Dl0.chips.all");
        QQuickItem *none = item("Dl0.chips.none");
        if (all && none) {
            QMetaObject::invokeMethod(all, "clicked");
            QMetaObject::invokeMethod(none, "clicked");
        }
        d.check("dl0: All and None ask setAll(true) and setAll(false)", all && none && w->property("alls").toInt() == 11);
        // Keyboard focus: the focus ring is the outline that stays
        c1->forceActiveFocus(Qt::TabFocusReason);
        d.spin(30);
        d.check("dl0: a chip with the keyboard focus shows the focus ring", borderWidth(background(c1)) == 2);
    }
    QQuickItem *rms = item("Dl0.one.chip.1");
    if (rms) QMetaObject::invokeMethod(rms, "clicked");
    d.spin(30);
    d.check("dl0: one-of chips: chosen(index), the radio role, no All / None",
            rms && w->property("chosenIndex").toInt() == 1 && rms->property("checked").toBool()
            && attached(rms, "Accessible.role").toInt() == QAccessible::RadioButton && item("Dl0.one.all") && !item("Dl0.one.all")->isVisible());

    QQuickItem *toggle = item("Dl0.toggle");
    if (toggle) QMetaObject::invokeMethod(toggle, "toggle");
    d.check("dl0: a toggle chip toggles", toggle && toggle->property("checked").toBool());

    // Segments: the bar keeps to its segments, the chosen one not accent-filled
    QQuickItem *segments = item("Dl0.segments");
    QQuickItem *bar = segments && !segments->childItems().isEmpty() ? segments->childItems().first() : nullptr;
    d.check("dl0: the segment bar is content-sized in a wider row",
            bar && segments->width() > bar->width() + 20 && qAbs(bar->width() - segments->implicitWidth()) < 1);
    QQuickItem *chosen = item("Dl0.segments.1");
    d.check("dl0: the chosen segment is the selection tone",
            chosen && background(chosen) && background(chosen)->property("color").value<QColor>() == theme->property("selection").value<QColor>());

    // Notice, empty state, rows, form, graph frame
    QQuickItem *notice = item("Dl0.notice.action");
    if (notice) QMetaObject::invokeMethod(notice, "clicked");
    QQuickItem *emptyAction = item("Dl0.empty.action");
    if (emptyAction) QMetaObject::invokeMethod(emptyAction, "clicked");
    d.check("dl0: the notice's and the empty state's actions fire", w->property("actions").toInt() == 11);
    QQuickItem *notice0 = item("Dl0.notice");
    d.check("dl0: a notice is at least 32 high, in the banner fill", notice0 && notice0->height() >= 32
            && attached(notice0, "Accessible.role").toInt() == QAccessible::AlertMessage);
    QQuickItem *row = item("Dl0.row.0");
    d.check("dl0: a list row is Theme.rowHeight high", row && qAbs(row->height() - 28) < 1);
    d.check("dl0: a list row has its accessible role and name",
            row && attached(row, "Accessible.role").toInt() == QAccessible::ListItem && attached(row, "Accessible.name").toString() == QLatin1String("Preset 1"));
    QQuickItem *form = item("Dl0.form");
    d.check("dl0: a form row is at least 36 high", form && form->height() >= 36);
    QQuickItem *column = item("Dl0.column");
    d.check("dl0: the content column stops at 760", column && column->width() <= 760.5);
    QQuickItem *labels = item("Dl0.graph.labels");
    int texts = 0;
    d.check("dl0: the graph frame's labels are whole inside the plot", labels && insideOf(labels, &texts) && texts == 6);
    QQuickItem *emptyGraph = item("Dl0.emptyGraph.empty");
    d.check("dl0: an empty graph frame says so, without its plot", emptyGraph && emptyGraph->isVisible()
            && item("Dl0.emptyGraph.plot") && !item("Dl0.emptyGraph.plot")->isVisible());
    QQuickItem *header = item("Dl0.header");
    d.check("dl0: the intro row is 44 high", header && qAbs(header->height() - 44) < 1);

    // -- WP DL0 fold --
    // The frame's grid is painted, not line items a label could sit on
    QQuickItem *graph = item("Dl0.graph");
    const QColor gridTone = theme->property("graphGrid").value<QColor>();
    int gridItems = 0, canvases = 0;
    const std::function<void(QQuickItem *)> walk = [&](QQuickItem *it) {
        for (QQuickItem *c : it->childItems()) {
            if (c->isVisible() && c->inherits("QQuickRectangle") && c->property("color").value<QColor>() == gridTone) ++gridItems;
            if (c->isVisible() && c->inherits("QQuickCanvasItem")) ++canvases;
            walk(c);
        }
    };
    if (graph) walk(graph);
    d.check("dl0 fold: the graph frame paints its grid (no grid-line items under the labels)", graph && gridItems == 0 && canvases == 1);
    // A row's hidden actions fade but stay enabled
    QQuickItem *del = item("Dl0.row.0.delete");
    QQuickItem *actions = del ? del->parentItem() : nullptr;
    d.spin(200);
    d.check("dl0 fold: a row's actions are hidden off hover, yet enabled", actions && actions->opacity() < 0.01 && del->isEnabled());
    if (del) QMetaObject::invokeMethod(del, "clicked");
    d.check("dl0 fold: and a scripted press reaches them", w->property("deletes").toInt() == 1);
    // A leading slot with nothing shown takes no room from the title
    QQuickItem *lead = item("Dl0.leadRow");
    QQuickItem *leadTitle = nullptr;
    const std::function<void(QQuickItem *)> findTitle = [&](QQuickItem *it) {
        for (QQuickItem *c : it->childItems()) {
            if (!leadTitle && c->inherits("QQuickText") && c->property("text").toString() == QLatin1String("Lead")) leadTitle = c;
            findTitle(c);
        }
    };
    if (lead) findTitle(lead);
    d.check("dl0 fold: an empty leading slot leaves the title at the left",
            leadTitle && leadTitle->mapToItem(lead, QPointF(0, 0)).x() < 40);
    // The status chip
    QQuickItem *status = item("Dl0.status");
    d.check("dl0 fold: a status chip: its dot in the state's colour, the status role, its words",
            status && status->property("dot").value<QColor>() == theme->property("warning").value<QColor>()
            && attached(status, "Accessible.role").toInt() == QAccessible::StatusBar
            && attached(status, "Accessible.name").toString() == QLatin1String("Meters stale"));
    // EmptyState's default icon is one the app ships
    QQuickItem *emptyDefault = item("Dl0.emptyDefault");
    const QString defaultName = emptyDefault ? emptyDefault->property("iconName").toString() : QString();
    d.check("dl0 fold: EmptyState's default icon is in the app's icons",
            !defaultName.isEmpty() && QFile::exists(QStringLiteral(":/qt/qml/UConsole/icons/lucide/%1.svg").arg(defaultName)));
}

void dl0Section(SmokeDriver &d) {
    auto *layout = Registrar::instance<LayoutController>();
    QObject *theme = d.app().engine->singletonInstance<QObject *>("UConsole", "Theme");
    d.check("dl0: the theme", layout && theme);
    if (!layout || !theme) return;
    const QString before = layout->layout();

    d.check("dl0: in Classic", d.switchLayout("classic"));
    d.check("dl0: Classic keeps its outlines and row height",
            theme->property("lineWidth").toInt() == 1 && theme->property("rowHeight").toInt() == 26
            && theme->property("radiusControl").toInt() == theme->property("radiusSmall").toInt());

    d.check("dl0: in Modern", d.switchLayout("modern"));
    QQmlComponent component(d.app().engine);
    component.setData(QByteArray(kProbe), QUrl(QStringLiteral("dl0.qml")));
    std::unique_ptr<QObject> probe(component.create());
    if (!probe) std::printf("smoke: dl0: %s\n", qPrintable(component.errorString()));
    auto *w = qobject_cast<QQuickWindow *>(probe.get());
    d.check("dl0: the probe window with every new component", w != nullptr);
    if (w) {
        d.waitFor(3000, [&] { return w->isExposed(); });
        d.spin(300);
        d.grab(w, QStringLiteral("dl0-components.png"));
        probeChecks(d, w, theme);
        w->close();
    }
    probe.reset();

    // The response graph: every axis label whole inside the plot
    d.goTo(QStringLiteral("equaliser/peq"), 300);
    QQuickItem *axis = d.item(QStringLiteral("GraphHost.plot.axisLabels"));
    int texts = 0;
    const bool inside = axis && axis->isVisible() && axis->width() > 100 && insideOf(axis, &texts);
    d.check("dl0: the response graph's axis labels are whole inside the plot (+25, -25, 20k)", inside && texts >= 10);
    QQuickItem *old = d.item(QStringLiteral("GraphHost.plot.labels"));
    d.check("dl0: BodePlot's cut labels are off in Modern", old && !old->isVisible());
    if (QQuickWindow *main = d.mainWindow()) d.grab(main, QStringLiteral("dl0-peq.png"));

    d.switchLayout(before == QLatin1String("modern") ? "modern" : "classic");
    QQuickItem *classicAxis = d.item(QStringLiteral("GraphHost.plot.axisLabels"));
    d.check("dl0: Classic keeps BodePlot's labels", !classicAxis || !classicAxis->isVisible());
}

const SmokeDriver::Section dl0("dl0-language", 985, &dl0Section);

} // namespace
