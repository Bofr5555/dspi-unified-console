// -- WP QP6 -- The matrix mixer on the demo device: the table's shape and
// names, crosspoints (connect, gain, INV, staged edits, the wheel, reset),
// the output row (ENABLE with the PDM / Core 1 questions, GAIN, DELAY with the
// device's line, MUTE), Direct 1:1 and Clear, the trims, the header menu
// (rename, copy / paste, identify), agreement with the output card's
// controller, and read-only firmware.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "MatrixController.h"
#include "MatrixModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QAccessible>
#include <QQmlContext>
#include <QQmlProperty>
#include <QQuickItem>
#include <QQuickWindow>
#include <QTest>
#include <QWheelEvent>
#include <cmath>

namespace {

bool near(double a, double b, double tol = 1e-3) { return std::abs(a - b) <= tol; }

struct Matrix {
    SmokeDriver &d;
    QQuickWindow *w;
    MatrixController *mc;
    ChannelController *cc;
    DeviceModel *device;

    const DspState *s() const { return device->state(); }
    QQuickItem *item(const QString &name) const { return d.item(name, w->contentItem()); }
    QObject *obj(const QString &name) const {
        if (QObject *o = w->findChild<QObject *>(name)) return o;
        return item(name);
    }
    void click(const QString &name, bool wait = true) {
        QObject *o = obj(name);
        d.check(QStringLiteral("qp6: %1 exists").arg(name), o != nullptr);
        if (o) QMetaObject::invokeMethod(o, "clicked");
        if (wait) d.waitIdle();
    }
    // Types into a MatrixField and presses Enter
    void type(const QString &name, const QString &text) {
        QObject *o = obj(name);
        d.check(QStringLiteral("qp6: %1 exists").arg(name), o != nullptr);
        if (!o) return;
        o->setProperty("text", text);
        QMetaObject::invokeMethod(o, "textEdited"); // -- WP INTE -- (REVE C1) as typed: Enter writes only typed text
        QMetaObject::invokeMethod(o, "accepted");
        d.waitIdle();
    }
    QPoint centre(QQuickItem *i) const { return i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint(); }
    // Scrolls an item into view, so pointer events reach it
    void reveal(QQuickItem *i) {
        QObject *scroll = obj(QStringLiteral("Matrix.scroll"));
        QQuickItem *flick = scroll ? scroll->property("contentItem").value<QQuickItem *>() : nullptr;
        if (!flick || !i) return;
        const QPointF inFlick = i->mapToItem(flick->property("contentItem").value<QQuickItem *>(), QPointF(0, 0));
        const double maxX = qMax(0.0, flick->property("contentWidth").toDouble() - flick->width());
        const double maxY = qMax(0.0, flick->property("contentHeight").toDouble() - flick->height());
        flick->setProperty("contentX", qBound(0.0, inFlick.x() - 40, maxX));
        flick->setProperty("contentY", qBound(0.0, inFlick.y() - 40, maxY));
        d.spin(60);
    }
    // Back to the table's top left (before a screenshot)
    void home() {
        QObject *scroll = obj(QStringLiteral("Matrix.scroll"));
        QQuickItem *flick = scroll ? scroll->property("contentItem").value<QQuickItem *>() : nullptr;
        if (flick) {
            flick->setProperty("contentX", 0);
            flick->setProperty("contentY", 0);
        }
        d.spin(60);
    }
    void wheel(QQuickItem *i, int notches) {
        reveal(i);
        const QPoint at = centre(i);
        QWheelEvent ev(at, w->mapToGlobal(at), QPoint(), QPoint(0, 120 * notches), Qt::NoButton, Qt::NoModifier,
                       Qt::NoScrollPhase, false);
        QCoreApplication::sendEvent(w, &ev);
        d.waitIdle();
    }
    void rightClick(QQuickItem *i) {
        reveal(i);
        QTest::mouseClick(w, Qt::RightButton, Qt::NoModifier, centre(i));
        d.waitIdle();
    }
    bool opened(QObject *popup) {
        return popup && d.waitFor(2000, [&]() { return popup->property("opened").toBool(); });
    }
    void answer(QObject *dialog, const char *a) {
        if (dialog) QMetaObject::invokeMethod(dialog, "finish", Q_ARG(QVariant, QString::fromLatin1(a)));
        d.waitFor(2000, [&]() { return dialog && !dialog->property("visible").toBool(); });
        d.waitIdle();
    }
    QString cellName(int i, int o, const char *part = nullptr) const {
        QString n = QStringLiteral("Matrix.cell.%1.%2").arg(i).arg(o);
        return part ? n + QLatin1Char('.') + QLatin1String(part) : n;
    }
    // The crosspoint as the device has it and as the matrix shows it
    bool deviceRoute(int i, int o, bool on, double gain, bool inv) const {
        return s() && s()->matrix_routing[i][o] == on && near(s()->matrix_gain[i][o], gain) && s()->matrix_invert[i][o] == inv;
    }
};

void shape(Matrix &m) {
    SmokeDriver &d = m.d;
    const ChannelListModel *list = m.device->channels();
    d.check(QStringLiteral("qp6: a row per input the sidebar shows"), m.mc->rows() == list->inputCount() && m.mc->rows() >= 2);
    d.check(QStringLiteral("qp6: a column per output"), m.mc->columns() == m.device->numOutputs() && m.mc->columns() >= 5);
    d.check(QStringLiteral("qp6: the model has rows × columns crosspoints"),
            m.mc->model()->rowCount() == m.mc->rows() * m.mc->columns());
    d.check(QStringLiteral("qp6: every crosspoint has a cell"), m.item(m.cellName(m.mc->rows() - 1, m.mc->columns() - 1)) != nullptr);

    // Headers: the sidebar's label (the core's derived "Out 1 · L · Low" or the user's name) and OUTn
    bool labels = true;
    for (int o = 0; o < m.mc->columns(); ++o) {
        QObject *h = m.obj(QStringLiteral("Matrix.header.%1").arg(o));
        const int row = list->rowOfChannel(m.cc->outputChannel(o));
        labels = labels && h && row >= 0 && h->property("shownName").toString() == list->get(row, QStringLiteral("label")).toString();
    }
    d.check(QStringLiteral("qp6: each header shows the output's label, as the sidebar does"), labels);
    d.check(QStringLiteral("qp6: headers carry OUTn, the last is the PDM output"),
            m.mc->column(0).value(QStringLiteral("descriptor")).toString() == QLatin1String("OUT1") &&
                m.mc->column(m.mc->pdmOutput()).value(QStringLiteral("pdm")).toBool());
    QObject *r0 = m.obj(QStringLiteral("Matrix.row.0"));
    d.check(QStringLiteral("qp6: row 1 is input 1's name"),
            r0 && r0->property("shownName").toString() == list->get(list->rowOfChannel(0), QStringLiteral("label")).toString());

    // The source-row rule (the upmixer's derived rows need a stereo input)
    d.check(QStringLiteral("qp6: rows: the inputs, or 3 / 5 while the upmixer derives rows"),
            MatrixController::sourceRows(8, false, true) == 8 && MatrixController::sourceRows(2, true, true) == 3 &&
                MatrixController::sourceRows(2, true, false) == 5 && MatrixController::sourceRows(2, false, false) == 2);
    d.check(QStringLiteral("qp6: the demo's eight inputs derive no upmix rows"), !m.mc->upmixRows());
    d.check(QStringLiteral("qp6: the window is the Mac's 960×660 by default"), m.w->width() >= 480 && m.w->height() >= 320);
}

void crosspoints(Matrix &m) {
    SmokeDriver &d = m.d;
    // A crosspoint that is off on the demo: input 2 into output 1
    const int i = 1, o = 0;
    if (m.s()->matrix_routing[i][o]) m.cc->setRoute(i, o, false, 0, false);
    d.waitIdle();
    QQuickItem *gainField = m.item(m.cellName(i, o, "gain"));
    QQuickItem *inv = m.item(m.cellName(i, o, "invert"));
    d.check(QStringLiteral("qp6: a disconnected cell shows neither gain nor INV"),
            gainField && inv && !gainField->isVisible() && !inv->isVisible());

    m.click(m.cellName(i, o, "connect"));
    d.check(QStringLiteral("qp6: a click connects the crosspoint"), m.deviceRoute(i, o, true, m.s()->matrix_gain[i][o], false));
    d.check(QStringLiteral("qp6: a connected cell shows its gain and INV"), gainField->isVisible() && inv->isVisible());

    m.type(m.cellName(i, o, "gain"), QStringLiteral("-3"));
    d.check(QStringLiteral("qp6: a typed crosspoint gain reaches the device"), m.deviceRoute(i, o, true, -3, false));
    // -- WP MX1 -- the tile shows the bare number
    d.check(QStringLiteral("qp6: the field shows \"-3\""), gainField->property("text").toString() == QLatin1String("-3"));
    m.type(m.cellName(i, o, "gain"), QStringLiteral("40"));
    d.check(QStringLiteral("qp6: a gain above +12 dB is held at +12"), m.deviceRoute(i, o, true, 12, false));
    m.type(m.cellName(i, o, "gain"), QStringLiteral("loud"));
    d.check(QStringLiteral("qp6: a text that is no number changes nothing"),
            m.deviceRoute(i, o, true, 12, false) && gainField->property("text").toString() == QLatin1String("+12"));
    m.type(m.cellName(i, o, "gain"), QStringLiteral("-1.5 dB"));
    d.check(QStringLiteral("qp6: the unit may be typed"),
            m.deviceRoute(i, o, true, -1.5, false) && gainField->property("text").toString() == QLatin1String("-1.5"));

    m.wheel(gainField, 1);
    d.check(QStringLiteral("qp6: a wheel notch raises the crosspoint gain by 0.5 dB"),
            m.deviceRoute(i, o, true, -1, false) && gainField->property("text").toString() == QLatin1String("-1"));
    // -- WP MX1 -- a right-click opens the cell's menu, which has the reset
    m.rightClick(gainField);
    QObject *cellMenu = m.obj(QStringLiteral("Matrix.cellMenu"));
    d.check(QStringLiteral("mx1: a right-click on a crosspoint opens its menu"), m.opened(cellMenu));
    // -- WP REVDFIX -- (REVD C2) MX1 ended the instant reset: the right-click
    // alone leaves the gain, and the menu's item is pressed as a person would
    // (click() does nothing on a disabled item; "triggered" ignored that)
    d.waitIdle();
    d.check(QStringLiteral("mx1: the right-click alone leaves the gain"), m.deviceRoute(i, o, true, -1, false)
            && gainField->property("text").toString() == QLatin1String("-1"));
    QObject *reset = m.obj(QStringLiteral("Matrix.cellMenu.reset"));
    d.check(QStringLiteral("mx1: the menu's Gain to 0 dB is enabled"), reset && reset->property("enabled").toBool());
    if (reset) QMetaObject::invokeMethod(reset, "click");
    if (cellMenu) cellMenu->setProperty("visible", false);
    d.waitIdle();
    d.check(QStringLiteral("qp6: the menu resets the crosspoint gain to 0 dB"), m.deviceRoute(i, o, true, 0, false));

    m.click(m.cellName(i, o, "invert"));
    d.check(QStringLiteral("qp6: INV inverts the crosspoint"), m.deviceRoute(i, o, true, 0, true));
    m.click(m.cellName(i, o, "invert"));
    m.click(m.cellName(i, o, "connect"));
    d.check(QStringLiteral("qp6: a second click disconnects, gain and polarity kept"), m.deviceRoute(i, o, false, 0, false));

    // Connect then INV at once, before the device answers: the INV carries the connect
    const int before = m.device->settersRun();
    m.click(m.cellName(i, o, "connect"), false);
    d.check(QStringLiteral("qp6: the cell shows the connect before the device answers"),
            m.mc->model()->get(i, o, QStringLiteral("connected")).toBool() && inv->isVisible());
    m.click(m.cellName(i, o, "invert"), false);
    d.waitIdle();
    d.check(QStringLiteral("qp6: connect and INV in quick succession both land"), m.deviceRoute(i, o, true, 0, true));
    d.check(QStringLiteral("qp6: and coalesce into one or two writes"), m.device->settersRun() - before <= 2);
    d.check(QStringLiteral("qp6: nothing stays staged once the device is idle"), m.mc->model()->stagedCount() == 0);

    // The output card's controller and the matrix read and write the same crosspoint
    const QVariantMap r = m.cc->route(i, o);
    d.check(QStringLiteral("qp6: the output card reads what the matrix wrote"),
            r.value(QStringLiteral("enabled")).toBool() && r.value(QStringLiteral("invert")).toBool());
    m.cc->setRoute(i, o, true, -6, false);
    d.waitIdle();
    d.check(QStringLiteral("qp6: the matrix shows what the output card wrote"),
            near(m.mc->model()->get(i, o, QStringLiteral("gain")).toDouble(), -6) &&
                !m.mc->model()->get(i, o, QStringLiteral("invert")).toBool() &&
                gainField->property("text").toString() == QLatin1String("-6"));
    m.cc->setRoute(i, o, false, 0, false);
    d.waitIdle();
}

void outputRow(Matrix &m) {
    SmokeDriver &d = m.d;
    const int o = 1;
    // GAIN
    m.type(QStringLiteral("Matrix.gain.%1").arg(o), QStringLiteral("-4.5"));
    d.check(QStringLiteral("qp6: GAIN sets the output's gain"), near(m.s()->output_gain_db[o], -4.5));
    // -- WP QPI2 -- the output's range is the device's, +20 dB as on the card
    m.type(QStringLiteral("Matrix.gain.%1").arg(o), QStringLiteral("18"));
    d.check(QStringLiteral("qpi2: the OUTPUT GAIN takes +18 dB, past the crosspoints' +12"), near(m.s()->output_gain_db[o], 18));
    m.type(QStringLiteral("Matrix.gain.%1").arg(o), QStringLiteral("25"));
    d.check(QStringLiteral("qpi2: and holds a gain above +20 dB at +20"), near(m.s()->output_gain_db[o], 20));
    m.type(QStringLiteral("Matrix.gain.%1").arg(o), QStringLiteral("-4.5"));
    // DELAY: kept as typed, past the device's line refused
    m.type(QStringLiteral("Matrix.delay.%1").arg(o), QStringLiteral("2.5"));
    d.check(QStringLiteral("qp6: DELAY keeps a typed 2.5 ms"), near(m.s()->output_delay_ms[o], 2.5));
    QQuickItem *delay = m.item(QStringLiteral("Matrix.delay.%1").arg(o));
    // -- WP MX1 -- the footer shows the bare number, its unit in the row's label
    d.check(QStringLiteral("qp6: the delay shows \"2.5\""), delay && delay->property("text").toString() == QLatin1String("2.5"));
    QString refused;
    QMetaObject::Connection c = QObject::connect(m.device, &DeviceModel::writeFailed, [&](const QString &what) { refused = what; });
    m.type(QStringLiteral("Matrix.delay.%1").arg(o), QString::number(m.cc->maxDelayMs() + 20));
    QObject::disconnect(c);
    d.check(QStringLiteral("qp6: a delay past the line is refused by the core"),
            refused == QStringLiteral("output delay %1").arg(o) && near(m.s()->output_delay_ms[o], 2.5));
    d.check(QStringLiteral("qp6: and the field shows the delay kept"), delay->property("text").toString() == QLatin1String("2.5"));
    m.wheel(delay, 1);
    d.check(QStringLiteral("qp6: a wheel notch moves 2.5 ms to the next whole ms"), near(m.s()->output_delay_ms[o], 3));
    m.wheel(delay, -1);
    d.check(QStringLiteral("qp6: and back down by 1 ms"), near(m.s()->output_delay_ms[o], 2));
    m.type(QStringLiteral("Matrix.delay.%1").arg(o), QStringLiteral("0"));
    // MUTE
    const bool muted = m.s()->output_muted[o];
    m.click(QStringLiteral("Matrix.mute.%1").arg(o));
    d.check(QStringLiteral("qp6: MUTE toggles the output"), m.s()->output_muted[o] == !muted);
    QObject *mute = m.obj(QStringLiteral("Matrix.mute.%1").arg(o));
    d.check(QStringLiteral("qp6: the mute button names its state"),
            mute && mute->property("toolTip").toString() == (muted ? QLatin1String("Mute") : QLatin1String("Unmute")));
    m.click(QStringLiteral("Matrix.mute.%1").arg(o));
    m.type(QStringLiteral("Matrix.gain.%1").arg(o), QStringLiteral("0"));
}

void enableQuestions(Matrix &m) {
    SmokeDriver &d = m.d;
    const int pdm = m.mc->pdmOutput();
    QObject *ask = m.obj(QStringLiteral("Matrix.enableQuestion"));
    d.check(QStringLiteral("qp6: the enable question exists"), ask != nullptr);
    if (!ask) return;
    // Start: PDM off, output 3 on
    if (m.s()->output_enabled[pdm]) m.cc->requestOutputEnable(pdm, false);
    if (!m.s()->output_enabled[2]) m.cc->requestOutputEnable(2, true);
    d.waitIdle();

    m.click(QStringLiteral("Matrix.enable.2"));
    d.check(QStringLiteral("qp6: ENABLE switches an output off without asking"),
            !m.s()->output_enabled[2] && !ask->property("visible").toBool());
    d.check(QStringLiteral("qp6: its column is dimmed"),
            !m.mc->model()->get(0, 2, QStringLiteral("outputEnabled")).toBool() &&
                m.item(m.cellName(0, 2))->opacity() < 0.5);
    m.click(QStringLiteral("Matrix.enable.2"));
    d.check(QStringLiteral("qp6: and on again without asking while PDM is off"), m.s()->output_enabled[2] && !ask->property("visible").toBool());

    // PDM while EQ outputs run
    d.check(QStringLiteral("qp6: the PDM column is outlined while enabling it would collide"),
            m.mc->model()->get(0, pdm, QStringLiteral("conflict")).toBool() &&
                m.mc->column(pdm).value(QStringLiteral("enableTip")).toString() == QLatin1String("Conflict: shared Core 1 resource"));
    const int before = m.device->settersRun();
    m.click(QStringLiteral("Matrix.enable.%1").arg(pdm));
    d.check(QStringLiteral("qp6: enabling PDM asks the Mac's question"),
            m.opened(ask) && ask->property("text").toString() == QStringLiteral("Outputs 3-%1 will be disabled. Are you sure?").arg(pdm) &&
                ask->property("acceptText").toString() == QLatin1String("Enable PDM"));
    d.check(QStringLiteral("qp6: nothing is sent before the answer"), m.device->settersRun() == before);
    m.answer(ask, "reject");
    d.check(QStringLiteral("qp6: Cancel changes nothing"), !m.s()->output_enabled[pdm] && m.s()->output_enabled[2]);
    m.click(QStringLiteral("Matrix.enable.%1").arg(pdm));
    m.opened(ask);
    m.answer(ask, "accept");
    bool eqOff = true;
    for (int e = 2; e < pdm; ++e) eqOff = eqOff && !m.s()->output_enabled[e];
    d.check(QStringLiteral("qp6: Enable PDM turns PDM on and outputs 3 to %1 off").arg(pdm), m.s()->output_enabled[pdm] && eqOff);
    d.check(QStringLiteral("qp6: the enabled PDM column is no longer outlined"), !m.mc->model()->get(0, pdm, QStringLiteral("conflict")).toBool());
    d.check(QStringLiteral("qp6: the ENABLE tooltip of an off EQ output names the conflict"),
            m.mc->column(3).value(QStringLiteral("enableTip")).toString() == QLatin1String("Conflict: shared Core 1 resource"));

    m.click(QStringLiteral("Matrix.enable.3"));
    d.check(QStringLiteral("qp6: enabling an EQ output under PDM asks the other question"),
            m.opened(ask) && ask->property("text").toString() == QLatin1String("The PDM output will be disabled. Are you sure?") &&
                ask->property("acceptText").toString() == QLatin1String("Disable PDM"));
    m.answer(ask, "accept");
    d.check(QStringLiteral("qp6: Disable PDM turns PDM off and output 4 on"), m.s()->output_enabled[3] && !m.s()->output_enabled[pdm]);
    d.check(QStringLiteral("qp6: the tooltip of an enabled output"),
            m.mc->column(3).value(QStringLiteral("enableTip")).toString() == QLatin1String("Disable output (saves CPU)"));
    m.home();
    d.check(QStringLiteral("qp6: screenshot of the matrix"), d.grab(m.w, QStringLiteral("window-matrix.png")));
}

void quickRoutes(Matrix &m) {
    SmokeDriver &d = m.d;
    QQuickItem *direct = m.item(QStringLiteral("Matrix.direct"));
    d.check(QStringLiteral("qp6: Direct 1:1 shows exactly in the 8-channel layout"), direct && direct->isVisible() == m.mc->wide());
    if (!m.mc->wide()) return;
    const int inputs = m.device->channels()->inputCount(), outputs = m.mc->columns(), pdm = m.mc->pdmOutput();
    // Something off the diagonal, PDM on, an inverted diagonal point
    m.cc->setRoute(0, 1, true, -2, false);
    m.cc->setRoute(1, 1, true, 3, true);
    m.cc->confirmOutputEnable(pdm);
    d.waitIdle();
    m.click(QStringLiteral("Matrix.direct"));
    bool diagonal = true;
    for (int i = 0; i < inputs; ++i)
        for (int o = 0; o < outputs; ++o) diagonal = diagonal && m.deviceRoute(i, o, i == o && i < outputs, 0, false);
    bool on = true;
    for (int o = 0; o < qMin(inputs, outputs); ++o) on = on && (o == pdm || m.s()->output_enabled[o]);
    d.check(QStringLiteral("qp6: Direct 1:1 routes each input to its output at 0 dB, nothing else"), diagonal);
    d.check(QStringLiteral("qp6: Direct 1:1 switches those outputs on and the PDM sub off"), on && !m.s()->output_enabled[pdm]);

    m.cc->setRoute(1, 2, true, -5, true);
    d.waitIdle();
    // -- WP MX1 -- past four crosspoints Clear asks first
    m.click(QStringLiteral("Matrix.clear"));
    QObject *clearAsk = m.obj(QStringLiteral("Matrix.clearQuestion"));
    d.check(QStringLiteral("mx1: Clear asks before it disconnects more than four"), m.opened(clearAsk));
    m.answer(clearAsk, "accept");
    bool none = true;
    for (int i = 0; i < inputs; ++i)
        for (int o = 0; o < outputs; ++o) none = none && !m.s()->matrix_routing[i][o];
    d.check(QStringLiteral("qp6: Clear disconnects every crosspoint"), none);
    d.check(QStringLiteral("qp6: and keeps their gain and polarity"), m.deviceRoute(1, 2, false, -5, true));
    m.home();
    d.check(QStringLiteral("qp6: screenshot of the cleared matrix"), d.grab(m.w, QStringLiteral("window-matrix-cleared.png")));

    // Trims (8-channel layout): input 3 is not linked by default
    QQuickItem *trim = m.item(QStringLiteral("Matrix.row.2.trim"));
    d.check(QStringLiteral("qp6: the 8-channel layout shows each input's trim"), trim && trim->isVisible());
    if (m.cc->isLinked(2)) m.cc->unlinkPair(2);
    m.type(QStringLiteral("Matrix.row.2.trim"), QStringLiteral("-2"));
    d.check(QStringLiteral("qp6: a typed trim sets the input's preamp"), near(m.s()->input_preamp_db[2], -2));
    m.type(QStringLiteral("Matrix.row.2.trim"), QStringLiteral("0"));
}

void headerMenu(Matrix &m) {
    SmokeDriver &d = m.d;
    const int o = 2;
    const int ch = m.cc->outputChannel(o);
    QObject *menu = m.obj(QStringLiteral("Matrix.columnMenu"));
    m.click(QStringLiteral("Matrix.header.%1.menu").arg(o));
    d.check(QStringLiteral("qp6: the header's chevron opens its menu"), m.opened(menu));
    QObject *identify = m.obj(QStringLiteral("Matrix.menu.identify"));
    auto *sc = Registrar::instance<SidebarController>();
    d.check(QStringLiteral("qp6: Identify shows when the generator can play it"),
            identify && identify->property("visible").toBool() == sc->identifySupported());
    d.check(QStringLiteral("qp6: Identify is enabled on an enabled output"),
            !sc->identifySupported() || identify->property("enabled").toBool() == bool(m.s()->output_enabled[o]));

    // Rename through the menu: the header turns into a field
    QMetaObject::invokeMethod(m.obj(QStringLiteral("Matrix.menu.rename")), "triggered");
    d.waitFor(2000, [&]() { return !menu->property("visible").toBool(); });
    QQuickItem *field = m.item(QStringLiteral("Matrix.header.%1.rename").arg(o));
    d.check(QStringLiteral("qp6: Rename turns the header into a field with the name"),
            field && field->isVisible() &&
                field->property("text").toString() ==
                    m.device->channels()->get(m.device->channels()->rowOfChannel(ch), QStringLiteral("name")).toString());
    if (field) {
        field->setProperty("text", QStringLiteral("  Sub Left  "));
        QMetaObject::invokeMethod(field, "textEdited"); // -- WP INTE -- (REVE C1) as typed: Enter writes only typed text
        QMetaObject::invokeMethod(field, "accepted");
    }
    d.waitIdle();
    QObject *header = m.obj(QStringLiteral("Matrix.header.%1").arg(o));
    d.check(QStringLiteral("qp6: the trimmed name reaches the device and the header"),
            header && header->property("shownName").toString() == QLatin1String("Sub Left") && field && !field->isVisible());
    d.check(QStringLiteral("qp6: an empty name changes nothing"), !m.mc->renameOutput(o, QStringLiteral("   ")));
    m.device->setChannelName(ch, QString());
    d.waitIdle();

    // Copy from output 1, paste into output 2
    m.cc->setOutputGain(0, -7);
    d.waitIdle();
    m.click(QStringLiteral("Matrix.header.0.menu"));
    m.opened(menu);
    QMetaObject::invokeMethod(m.obj(QStringLiteral("Matrix.menu.copy")), "triggered");
    d.waitIdle();
    d.check(QStringLiteral("qp6: Copy Parameters fills the channel clipboard"), sc->hasClipboard());
    m.click(QStringLiteral("Matrix.header.1.menu"));
    m.opened(menu);
    QObject *paste = m.obj(QStringLiteral("Matrix.menu.paste"));
    d.check(QStringLiteral("qp6: Paste Parameters is enabled with something copied"), paste && paste->property("enabled").toBool());
    QMetaObject::invokeMethod(paste, "triggered");
    d.waitIdle();
    d.check(QStringLiteral("qp6: Paste Parameters gives output 2 output 1's gain"), near(m.s()->output_gain_db[1], -7));
    menu->setProperty("visible", false);

    // Right-click on a header opens the menu too
    QQuickItem *h0 = m.item(QStringLiteral("Matrix.header.0"));
    m.rightClick(h0);
    d.check(QStringLiteral("qp6: a right-click on a header opens its menu"), m.opened(menu));
    menu->setProperty("visible", false);
}

QVariant attached(QObject *o, const char *name) {
    return QQmlProperty(o, QString::fromLatin1(name), qmlContext(o)).read();
}

void accessibility(Matrix &m) {
    SmokeDriver &d = m.d;
    const struct {
        QString name;
        QAccessible::Role role;
    } controls[] = { { m.cellName(0, 0, "connect"), QAccessible::CheckBox },
                     { QStringLiteral("Matrix.enable.0"), QAccessible::CheckBox },
                     { QStringLiteral("Matrix.mute.0"), QAccessible::CheckBox },
                     { QStringLiteral("Matrix.gain.0"), QAccessible::EditableText },
                     { QStringLiteral("Matrix.direct"), QAccessible::Button } };
    for (const auto &c : controls) {
        QObject *o = m.obj(c.name);
        d.check(QStringLiteral("qp6: %1 has its accessible role and a name").arg(c.name),
                o && attached(o, "Accessible.role").toInt() == int(c.role) && !attached(o, "Accessible.name").toString().isEmpty());
    }
    QObject *dot = m.obj(m.cellName(1, 0, "connect"));
    const QString in1 = m.mc->row(1).value(QStringLiteral("name")).toString();
    d.check(QStringLiteral("qp6: a crosspoint is named by its input and output"),
            dot && attached(dot, "Accessible.name").toString().startsWith(in1 + QStringLiteral(" to ")));
}

void readOnly(Matrix &m) {
    SmokeDriver &d = m.d;
    m.d.app().core->worker()->post([](FfiCore *c) { dspi_demo_open_wire(c, 17); });
    m.device->refresh();
    d.check(QStringLiteral("qp6: the V17 recording opens read-only"), d.waitFor(5000, [&]() { return m.device->readOnly(); }));
    d.waitIdle();
    d.spin(100);
    d.check(QStringLiteral("qp6: read-only: the matrix is not writable"), !m.mc->writable());
    bool disabled = true;
    for (const QString &n : { m.cellName(0, 0, "connect"), QStringLiteral("Matrix.enable.0"), QStringLiteral("Matrix.gain.0"),
                              QStringLiteral("Matrix.delay.0"), QStringLiteral("Matrix.mute.0") }) {
        QQuickItem *i = m.item(n);
        disabled = disabled && i && !i->isEnabled();
    }
    d.check(QStringLiteral("qp6: read-only disables crosspoints, ENABLE, GAIN, DELAY and MUTE"), disabled);
    const int before = m.device->settersRun();
    m.click(m.cellName(0, 0, "connect"));
    m.mc->setConnected(0, 1, true);
    m.mc->toggleEnable(0);
    m.mc->setOutputGain(0, -9);
    m.mc->directRouting();
    m.mc->clearRoutes();
    d.waitIdle();
    d.check(QStringLiteral("qp6: read-only sends nothing"), m.device->settersRun() == before);
    m.home();
    d.check(QStringLiteral("qp6: screenshot of a read-only matrix"), d.grab(m.w, QStringLiteral("window-matrix-readonly.png")));
}

void matrixSection(SmokeDriver &d) {
    Shell *shell = d.app().shell;
    shell->trigger(QStringLiteral("tools.matrix"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("tools.matrix")));
    d.check(QStringLiteral("qp6: Tools > Matrix Mixer opens the window"), w && d.waitFor(3000, [&]() { return w->isVisible(); }));
    d.check(QStringLiteral("qp6: its title is the Mac's"), w && w->title() == QLatin1String("Matrix Mixer"));
    auto *mc = Registrar::instance<MatrixController>();
    auto *cc = Registrar::instance<ChannelController>();
    d.check(QStringLiteral("qp6: the matrix controller is registered"), mc && cc);
    if (!w || !mc || !cc) return;
    d.waitIdle();
    d.spin(100);
    Matrix m{ d, w, mc, cc, d.app().device };
    d.check(QStringLiteral("qp6: the demo is writable"), mc->writable());

    shape(m);
    crosspoints(m);
    outputRow(m);
    enableQuestions(m);
    quickRoutes(m);
    headerMenu(m);
    accessibility(m);
    readOnly(m);

    // Back to the demo as it opens, for the sections after this one
    d.app().device->openDemo();
    d.check(QStringLiteral("qp6: the demo reopens writable"),
            d.waitFor(5000, [&]() { return !d.app().device->readOnly() && d.app().device->connected(); }));
    d.waitIdle();
    w->close();
}

const SmokeDriver::Section matrix("qp6-matrix", 60, &matrixSection);

} // namespace
