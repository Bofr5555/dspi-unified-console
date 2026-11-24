// -- WP MX1 -- The matrix mixer redrawn (DESREV's matrix-mockup.html), on the
// demo: the tiles, the corner, tails and footers; the headers held while the
// grid scrolls; the keyboard; Link L/R pairs (the mirrored write as one step);
// the inspector's three states and its summary; Clear's question; the dense
// grid; Modern's page and Classic's window, which show the same panel.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "HistoryController.h"
#include "LayoutController.h"
#include "MatrixController.h"
#include "MatrixModel.h"
#include "Registrar.h"
#include "Shell.h"
#include "SidebarController.h"

#include <QGuiApplication>
#include <QKeyEvent>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTest>
#include <cmath>

namespace {

bool near(double a, double b, double tol = 1e-3) { return std::abs(a - b) <= tol; }

struct Mx {
    SmokeDriver &d;
    QQuickWindow *w = nullptr;   // the window the panel is in
    MatrixController *mc;
    ChannelController *cc;
    SidebarController *sc;
    DeviceModel *device;

    void check(const QString &what, bool ok) const { d.check(QStringLiteral("mx1: ") + what, ok); }
    const DspState *s() const { return device->state(); }
    // The shown item named `name` in the window
    QQuickItem *shown(const QString &name, QQuickItem *from = nullptr) const {
        QQuickItem *i = from ? from : w->contentItem();
        if (!i || !i->isVisible()) return nullptr;
        if (i->objectName() == name) return i;
        for (QQuickItem *c : i->childItems())
            if (QQuickItem *f = shown(name, c)) return f;
        return nullptr;
    }
    QQuickItem *any(const QString &name) const { return d.item(name, w->contentItem()); }
    QObject *popup(const QString &name) const {
        if (QObject *o = d.object(name)) return o;
        QQuickItem *panel = any(QStringLiteral("MatrixPanel"));
        return panel ? panel->findChild<QObject *>(name) : nullptr;
    }
    void settle() const {
        d.spin(40);
        d.waitIdle();
        d.spin(60);
    }
    static QPoint centre(QQuickItem *i) { return i->mapToScene(QPointF(i->width() / 2, i->height() / 2)).toPoint(); }
    bool click(const QString &name) const {
        QQuickItem *i = shown(name);
        if (!i || i->width() <= 0) return false;
        QTest::mouseClick(w, Qt::LeftButton, Qt::NoModifier, centre(i));
        settle();
        return true;
    }
    void key(int k, Qt::KeyboardModifiers m = Qt::NoModifier) const {
        QTest::keyClick(w, Qt::Key(k), m);
        settle();
    }
    // Typed into what has the focus, a key event per character (QTest has no keyClicks for a QWindow)
    void typeKeys(const QString &text) const {
        for (const QChar c : text) {
            const int k = c.isDigit() ? Qt::Key_0 + c.digitValue() : c == QLatin1Char('-') ? Qt::Key_Minus
                        : c == QLatin1Char(',') ? Qt::Key_Comma : c == QLatin1Char('.') ? Qt::Key_Period : Qt::Key_unknown;
            QKeyEvent press(QEvent::KeyPress, k, Qt::NoModifier, QString(c));
            QKeyEvent release(QEvent::KeyRelease, k, Qt::NoModifier, QString(c));
            QCoreApplication::sendEvent(w, &press);
            QCoreApplication::sendEvent(w, &release);
        }
        d.spin(30);
    }
    QQuickItem *grid() const { return any(QStringLiteral("Matrix.grid")); }
    bool focusGrid() const {
        QQuickItem *g = grid();
        if (!g) return false;
        w->requestActivate();
        d.waitFor(1000, [&] { return QGuiApplication::focusWindow() == w; });
        // Away first (a scope above would give it back), so the grid takes the focus anew
        g->setFocus(false);
        d.spin(10);
        g->forceActiveFocus(Qt::TabFocusReason);
        d.spin(30);
        return gridHasKeys();
    }
    // The grid itself has the keyboard, not a field in it
    bool gridHasKeys() const { return grid() && w->activeFocusItem() == grid(); }
    bool at(int i, int o) const { return mc->currentInput() == i && mc->currentOutput() == o; }
    bool route(int i, int o, bool on, double gain, bool inv) const {
        return s() && s()->matrix_routing[i][o] == on && near(s()->matrix_gain[i][o], gain) && s()->matrix_invert[i][o] == inv;
    }
    bool connected(int i, int o) const { return s() && s()->matrix_routing[i][o]; }
    // Every crosspoint of a few rows and columns off, at 0 dB and normal phase
    void clean() {
        for (int i = 0; i < qMin(4, mc->rows()); ++i)
            for (int o = 0; o < qMin(6, mc->columns()); ++o)
                if (!route(i, o, false, 0, false)) cc->setRoute(i, o, false, 0, false);
        d.waitIdle();
        settle();
    }
};

// ---- The grid as drawn -------------------------------------------------------------

void drawn(Mx &m, const QString &where) {
    QQuickItem *cell = m.shown(QStringLiteral("Matrix.cell.0.0"));
    m.check(where + QStringLiteral(": a cell is 56 × 40"), cell && near(cell->width(), 56) && near(cell->height(), 40));
    QQuickItem *corner = m.shown(QStringLiteral("Matrix.corner"));
    m.check(where + QStringLiteral(": the corner says which way the signal goes"), corner != nullptr);
    // Input 1 into outputs 1 and 3, nothing else in its row
    m.clean();
    m.cc->setRoute(0, 0, true, 0, false);
    m.cc->setRoute(0, 2, true, -6, true);
    for (int o = 3; o < m.mc->columns(); ++o)
        if (m.connected(0, o)) m.cc->setRoute(0, o, false, 0, false);
    m.d.waitIdle();
    m.settle();
    QQuickItem *tail = m.shown(QStringLiteral("Matrix.tail.0"));
    m.check(where + QStringLiteral(": the row's tail lists what it feeds (%1)").arg(tail ? tail->property("text").toString() : QString()),
            tail && tail->property("text").toString() == QStringLiteral("→ Out 1, 3"));
    QQuickItem *tail1 = m.shown(QStringLiteral("Matrix.tail.1"));
    m.check(where + QStringLiteral(": a row that feeds nothing says so"),
            tail1 && (m.mc->model()->feeds(1).isEmpty() ? tail1->property("text").toString() == QStringLiteral("→ nothing") : true));
    QQuickItem *gain = m.shown(QStringLiteral("Matrix.cell.0.2.gain"));
    m.check(where + QStringLiteral(": the gain is inside the tile, without its unit"), gain && gain->property("text").toString() == QLatin1String("-6"));
    QQuickItem *inv = m.shown(QStringLiteral("Matrix.cell.0.2.invert"));
    m.check(where + QStringLiteral(": an inverted crosspoint shows its ø"), inv && inv->opacity() == 1);
    QQuickItem *empty = m.any(QStringLiteral("Matrix.cell.1.2.gain"));
    m.check(where + QStringLiteral(": an empty cell shows no number"), empty && !empty->isVisible());
    QQuickItem *cell02 = m.shown(QStringLiteral("Matrix.cell.0.2"));
    m.check(where + QStringLiteral(": a connected cell keeps its size"), cell02 && near(cell02->width(), 56) && near(cell02->height(), 40));
    QQuickItem *fed = m.shown(QStringLiteral("Matrix.fedBy.2"));
    m.check(where + QStringLiteral(": the footer says what feeds an output"),
            fed && fed->property("sources").toList().contains(QVariant(0)));
    m.check(where + QStringLiteral(": the output's gain and delay sit under its column"),
            m.shown(QStringLiteral("Matrix.gain.2")) && m.shown(QStringLiteral("Matrix.delay.2")) && m.shown(QStringLiteral("Matrix.row.2.trim")));
    QQuickItem *header = m.shown(QStringLiteral("Matrix.header.2"));
    QQuickItem *enable = m.shown(QStringLiteral("Matrix.enable.2"));
    QQuickItem *mute = m.shown(QStringLiteral("Matrix.mute.2"));
    const auto inside = [](QQuickItem *i, QQuickItem *outer) {
        if (!i || !outer) return false;
        const QRectF r = i->mapRectToItem(outer, QRectF(0, 0, i->width(), i->height()));
        return r.left() >= -0.5 && r.top() >= -0.5 && r.right() <= outer->width() + 0.5 && r.bottom() <= outer->height() + 0.5;
    };
    m.check(where + QStringLiteral(": enable and mute sit in the column's header"), inside(enable, header) && inside(mute, header));
}

// The headers stay while the grid scrolls under them
void sticky(Mx &m, const QString &where) {
    QObject *scroll = m.any(QStringLiteral("Matrix.scroll"));
    QQuickItem *flick = scroll ? scroll->property("contentItem").value<QQuickItem *>() : nullptr;
    QQuickItem *header = m.any(QStringLiteral("Matrix.header.1"));
    QQuickItem *row = m.any(QStringLiteral("Matrix.row.1"));
    QQuickItem *cell = m.any(QStringLiteral("Matrix.cell.1.1"));
    m.check(where + QStringLiteral(": the grid scrolls here"), flick && header && row && cell
            && flick->property("contentHeight").toDouble() > flick->height() && flick->property("contentWidth").toDouble() > flick->width());
    if (!flick || !header || !row || !cell) return;
    const QPointF h0 = header->mapToScene(QPointF()), r0 = row->mapToScene(QPointF()), c0 = cell->mapToScene(QPointF());
    flick->setProperty("contentX", 30);
    flick->setProperty("contentY", 20);
    m.d.spin(60);
    const QPointF h1 = header->mapToScene(QPointF()), r1 = row->mapToScene(QPointF()), c1 = cell->mapToScene(QPointF());
    m.check(where + QStringLiteral(": the column headers stay at the top and follow the columns"), near(h1.y(), h0.y()) && near(h1.x(), h0.x() - 30));
    m.check(where + QStringLiteral(": the row headers stay at the left and follow the rows"), near(r1.x(), r0.x()) && near(r1.y(), r0.y() - 20));
    m.check(where + QStringLiteral(": the cells move under both"), near(c1.x(), c0.x() - 30) && near(c1.y(), c0.y() - 20));
    flick->setProperty("contentX", 0);
    flick->setProperty("contentY", 0);
    m.d.spin(60);
}

// ---- The keyboard ------------------------------------------------------------------

void keys(Mx &m, const QString &where) {
    m.clean();
    m.mc->setCurrent(-1, -1);
    m.check(where + QStringLiteral(": Tab puts the grid on its first cell"), m.focusGrid() && m.at(0, 0));
    m.key(Qt::Key_Right);
    m.key(Qt::Key_Down);
    m.check(where + QStringLiteral(": the arrows move"), m.at(1, 1));
    m.key(Qt::Key_End);
    m.check(where + QStringLiteral(": End goes to the last column"), m.at(1, m.mc->columns() - 1));
    m.key(Qt::Key_Home);
    m.check(where + QStringLiteral(": Home to the first"), m.at(1, 0));
    m.key(Qt::Key_PageDown);
    m.check(where + QStringLiteral(": PgDn to the last row"), m.at(m.mc->rows() - 1, 0));
    m.key(Qt::Key_PageUp);
    m.check(where + QStringLiteral(": PgUp to the first"), m.at(0, 0));
    m.key(Qt::Key_Left);
    m.key(Qt::Key_Up);
    m.check(where + QStringLiteral(": and stop at the edges"), m.at(0, 0));
    m.key(Qt::Key_Down);
    m.key(Qt::Key_Right);
    m.check(where + QStringLiteral(": on 2 → 2"), m.at(1, 1));

    m.key(Qt::Key_Space);
    m.check(where + QStringLiteral(": Space connects at 0 dB"), m.route(1, 1, true, 0, false));
    m.key(Qt::Key_Return);
    QQuickItem *field = m.any(QStringLiteral("Matrix.cell.1.1.gain"));
    m.check(where + QStringLiteral(": Enter types the gain in the tile"), field && field->hasActiveFocus());
    m.typeKeys(QStringLiteral("-6"));
    m.key(Qt::Key_Return);
    m.check(where + QStringLiteral(": Enter commits it"), m.route(1, 1, true, -6, false));
    m.check(where + QStringLiteral(": and gives the keyboard back to the grid"), m.gridHasKeys() && m.at(1, 1));
    m.key(Qt::Key_3);
    m.check(where + QStringLiteral(": a digit starts typing with it"), field && field->hasActiveFocus() && field->property("text").toString() == QLatin1String("3"));
    m.typeKeys(QStringLiteral(",5"));
    m.key(Qt::Key_Return);
    m.check(where + QStringLiteral(": a decimal comma is read (USR1)"), m.route(1, 1, true, 3.5, false));
    m.key(Qt::Key_Return);
    m.typeKeys(QStringLiteral("-20"));
    m.key(Qt::Key_Escape);
    m.check(where + QStringLiteral(": Escape puts the gain back"), m.route(1, 1, true, 3.5, false) && m.gridHasKeys());
    m.check(where + QStringLiteral(": and the tile shows it"), field && field->property("text").toString() == QLatin1String("+3.5"));
    m.key(Qt::Key_Minus);
    m.check(where + QStringLiteral(": - steps down 0.5 dB"), m.route(1, 1, true, 3, false));
    m.key(Qt::Key_Plus);
    m.key(Qt::Key_Plus);
    m.check(where + QStringLiteral(": + steps up 0.5 dB"), m.route(1, 1, true, 4, false));
    m.key(Qt::Key_Minus, Qt::ControlModifier);
    m.check(where + QStringLiteral(": Ctrl+- steps 3 dB"), m.route(1, 1, true, 1, false));
    m.key(Qt::Key_I);
    m.check(where + QStringLiteral(": I inverts the phase"), m.route(1, 1, true, 1, true));
    m.key(Qt::Key_I);
    m.check(where + QStringLiteral(": and back"), m.route(1, 1, true, 1, false));
    m.key(Qt::Key_Delete);
    m.check(where + QStringLiteral(": Delete disconnects, the gain kept"), m.route(1, 1, false, 1, false));
    m.key(Qt::Key_5);
    m.key(Qt::Key_Return);
    m.check(where + QStringLiteral(": a digit on an empty cell connects it first"), m.route(1, 1, true, 5, false));

    // The column's output
    const bool muted = m.s()->output_muted[1];
    m.key(Qt::Key_M);
    m.check(where + QStringLiteral(": M mutes the column's output"), m.s()->output_muted[1] == !muted);
    m.key(Qt::Key_M);
    const int pdm = m.mc->pdmOutput();
    if (m.s()->output_enabled[pdm]) m.cc->requestOutputEnable(pdm, false);
    if (!m.s()->output_enabled[1]) m.cc->requestOutputEnable(1, true);
    m.d.waitIdle();
    m.key(Qt::Key_E);
    m.check(where + QStringLiteral(": E switches it off"), !m.s()->output_enabled[1]);
    QQuickItem *dim = m.any(QStringLiteral("Matrix.cell.0.1"));
    // 45 % in Classic; Modern keeps its text at AA contrast (Theme.dim)
    const double dimmed = Registrar::instance<LayoutController>()->modern() ? 0.75 : 0.45;
    m.check(where + QStringLiteral(": its column dims"), dim && near(dim->opacity(), dimmed));
    m.key(Qt::Key_E);
    m.check(where + QStringLiteral(": and on"), m.s()->output_enabled[1]);

    // The cell's menu
    m.key(Qt::Key_F10, Qt::ShiftModifier);
    QObject *menu = m.popup(QStringLiteral("Matrix.cellMenu"));
    m.check(where + QStringLiteral(": Shift+F10 opens the cell's menu"),
            menu && m.d.waitFor(2000, [&] { return menu->property("opened").toBool(); }));
    if (menu) {
        QObject *invert = m.popup(QStringLiteral("Matrix.cellMenu.invert"));
        if (invert) QMetaObject::invokeMethod(invert, "triggered");
        m.settle();
        m.check(where + QStringLiteral(": its Invert Phase inverts"), m.route(1, 1, true, 5, true));
        QMetaObject::invokeMethod(menu, "close");
        m.settle();
    }
    m.key(Qt::Key_Escape);
    m.check(where + QStringLiteral(": Escape leaves the cell"), m.at(-1, -1));
    m.cc->setRoute(1, 1, false, 0, false);
    m.d.waitIdle();
}

// A click on an empty cell connects; on a connected one it selects, then disconnects
void clicks(Mx &m, const QString &where) {
    m.clean();
    m.mc->setCurrent(-1, -1);
    m.check(where + QStringLiteral(": a click on an empty cell connects it"), m.click(QStringLiteral("Matrix.cell.2.3")) && m.route(2, 3, true, 0, false) && m.at(2, 3));
    m.mc->setCurrent(0, 0);
    m.click(QStringLiteral("Matrix.cell.2.3"));
    m.check(where + QStringLiteral(": on a connected one, it is chosen and stays connected"), m.at(2, 3) && m.route(2, 3, true, 0, false));
    m.click(QStringLiteral("Matrix.cell.2.3"));
    m.check(where + QStringLiteral(": and a click on the chosen one disconnects it"), m.route(2, 3, false, 0, false));
    QQuickItem *cell = m.shown(QStringLiteral("Matrix.cell.2.3"));
    if (cell) {
        QTest::mouseDClick(m.w, Qt::LeftButton, Qt::NoModifier, Mx::centre(cell));
        m.settle();
    }
    QQuickItem *field = m.any(QStringLiteral("Matrix.cell.2.3.gain"));
    m.check(where + QStringLiteral(": a double-click types the gain (connecting it)"), m.route(2, 3, true, 0, false) && field && field->hasActiveFocus());
    m.typeKeys(QStringLiteral("−2,5"));
    m.key(Qt::Key_Return);
    m.check(where + QStringLiteral(": \"−2,5\" with the minus sign and a comma is read (USR1)"), m.route(2, 3, true, -2.5, false));
    // -- WP USR1 -- typed, then the focus leaves: the typed value is sent, never the old one
    m.key(Qt::Key_Return);
    m.typeKeys(QStringLiteral("-4"));
    QQuickItem *other = m.shown(QStringLiteral("Matrix.cell.3.4"));
    if (other) QTest::mouseClick(m.w, Qt::LeftButton, Qt::NoModifier, Mx::centre(other));
    m.settle();
    m.check(where + QStringLiteral(": typed, then a click on an empty cell: the typed gain lands (USR1)"), m.route(2, 3, true, -4, false));
    m.check(where + QStringLiteral(": and the click connects that cell"), m.route(3, 4, true, 0, false));
    // While a gain is typed, a click on another tile types that one next
    m.mc->setCurrent(2, 3);
    m.key(Qt::Key_Return);
    m.typeKeys(QStringLiteral("-7"));
    QQuickItem *next = m.shown(QStringLiteral("Matrix.cell.3.4"));
    if (next) QTest::mouseClick(m.w, Qt::LeftButton, Qt::NoModifier, Mx::centre(next));
    m.settle();
    QQuickItem *nextField = m.any(QStringLiteral("Matrix.cell.3.4.gain"));
    m.check(where + QStringLiteral(": typed, then a click on another tile: the gain lands and that tile is typed in"),
            m.route(2, 3, true, -7, false) && nextField && nextField->hasActiveFocus() && m.route(3, 4, true, 0, false));
    m.key(Qt::Key_Escape);
    m.check(where + QStringLiteral(": Escape there changes nothing"), m.route(3, 4, true, 0, false) && !nextField->hasActiveFocus());
    m.clean();
}

// ---- Link L/R pairs ------------------------------------------------------------------

void linked(Mx &m, const QString &where, const QString &linkButton) {
    m.clean();
    m.mc->setLinkPairs(false);
    m.settle();
    m.check(where + QStringLiteral(": Link L/R pairs is off by default"), !m.mc->linkPairs());
    m.check(where + QStringLiteral(": pairs: L with R, the derived and the last odd row with nothing"),
            m.mc->pairRow(0) == 1 && m.mc->pairRow(1) == 0 && m.mc->pairRow(2) == 3);
    m.click(QStringLiteral("Matrix.cell.0.0"));
    m.check(where + QStringLiteral(": unlinked, a change stays on its crosspoint"), m.connected(0, 0) && !m.connected(1, 1));
    m.clean();

    m.check(where + QStringLiteral(": the tool switches it on"), m.click(linkButton) && m.mc->linkPairs());
    QQuickItem *mark = m.shown(QStringLiteral("Matrix.pairMark.0"));
    m.check(where + QStringLiteral(": a glyph joins the paired rows"), mark != nullptr);
    const QString serial = m.device->serial();
    m.check(where + QStringLiteral(": kept for the device"),
            serial.isEmpty() || QSettings().value(QStringLiteral("matrix/linkPairs/%1").arg(serial)).toBool());

    const int before = m.device->settersRun();
    m.click(QStringLiteral("Matrix.cell.0.2"));
    m.check(where + QStringLiteral(": L → Out 3 also connects R → Out 4"), m.route(0, 2, true, 0, false) && m.route(1, 3, true, 0, false));
    m.check(where + QStringLiteral(": in one write"), m.device->settersRun() - before == 1);
    m.check(where + QStringLiteral(": nothing else is touched"), !m.connected(1, 2) && !m.connected(0, 3));
    m.mc->setGain(0, 2, -4);
    m.mc->setInvert(0, 2, true);
    m.d.waitIdle();
    m.check(where + QStringLiteral(": gain and inversion are mirrored"), m.route(1, 3, true, -4, true));
    m.mc->setGain(1, 3, -2);
    m.d.waitIdle();
    m.check(where + QStringLiteral(": from R to L too"), m.route(0, 2, true, -2, true));
    m.mc->setConnected(0, 3, true);
    m.d.waitIdle();
    m.check(where + QStringLiteral(": inside the output pair: L → Out 4 mirrors to R → Out 3"), m.connected(0, 3) && m.connected(1, 2));
    m.check(where + QStringLiteral(": the last output has no partner"), m.mc->mirrorOf(0, m.mc->columns() - 1).isEmpty() || (m.mc->columns() % 2 == 0));
    m.check(where + QStringLiteral(": a row past the pairs is not mirrored"), m.mc->rows() % 2 == 0 || m.mc->mirrorOf(m.mc->rows() - 1, 0).isEmpty());

    // One undo step takes both back
    auto *history = Registrar::instance<HistoryController>();
    if (history) {
        m.mc->setConnected(0, 0, true);
        m.d.waitIdle();
        m.check(where + QStringLiteral(": (connected as a pair)"), m.connected(0, 0) && m.connected(1, 1));
        history->undo();
        m.d.waitIdle();
        m.d.waitFor(2000, [&] { return !m.connected(0, 0); });
        m.settle();
        m.check(where + QStringLiteral(": one undo takes the pair back"), !m.connected(0, 0) && !m.connected(1, 1));
    }

    // The inspector says what is mirrored
    m.mc->setCurrent(0, 2);
    m.settle();
    m.check(where + QStringLiteral(": the inspector names the mirrored crosspoint"),
            !m.d.app().shell->pageHost() || m.w != m.d.mainWindow() || m.shown(QStringLiteral("RoutingInspector.mirror")) != nullptr);

    m.click(linkButton);
    m.check(where + QStringLiteral(": and the tool switches it off"), !m.mc->linkPairs() && !m.shown(QStringLiteral("Matrix.pairMark.0")));
    m.mc->setCurrent(-1, -1);
    m.clean();
}

// ---- Clear asks past four crosspoints -------------------------------------------------

void clearQuestion(Mx &m, const QString &where, const QString &prefix) {
    m.clean();
    for (int i = 0; i < 3; ++i) m.cc->setRoute(i, i, true, 0, false);
    m.d.waitIdle();
    m.settle();
    if (m.mc->model()->connectedCount() <= 4) {
        m.click(prefix + QStringLiteral(".clear"));
        m.check(where + QStringLiteral(": four or fewer crosspoints clear at once"), m.mc->model()->connectedCount() == 0);
        for (int i = 0; i < 5; ++i) m.cc->setRoute(i, i, true, 0, false);
        m.d.waitIdle();
        m.settle();
    }
    QObject *ask = m.popup(prefix + QStringLiteral(".clearQuestion"));
    const int n = m.mc->model()->connectedCount();
    m.click(prefix + QStringLiteral(".clear"));
    m.check(where + QStringLiteral(": Clear asks first past four crosspoints"), ask && m.d.waitFor(2000, [&] { return ask->property("opened").toBool(); }));
    if (!ask) return;
    QMetaObject::invokeMethod(ask, "finish", Q_ARG(QVariant, QStringLiteral("reject")));
    m.settle();
    m.check(where + QStringLiteral(": Cancel keeps them"), m.mc->model()->connectedCount() == n);
    m.click(prefix + QStringLiteral(".clear"));
    m.d.waitFor(2000, [&] { return ask->property("opened").toBool(); });
    QMetaObject::invokeMethod(ask, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    m.settle();
    m.check(where + QStringLiteral(": Clear disconnects them all"), m.mc->model()->connectedCount() == 0);
    m.d.app().device->openDemo();
    m.d.waitFor(5000, [&] { return m.device->connected() && !m.device->readOnly(); });
    m.d.waitIdle();
    m.settle();
}

// ---- Dense ---------------------------------------------------------------------------

void dense(Mx &m, const QString &where, const QString &denseButton, const QString &shot) {
    m.check(where + QStringLiteral(": the tool chooses the dense grid"), m.click(denseButton) && m.mc->dense());
    m.settle();
    QQuickItem *cell = m.shown(QStringLiteral("Matrix.cell.0.0"));
    m.check(where + QStringLiteral(": dense cells are 44 × 32"), cell && near(cell->width(), 44) && near(cell->height(), 32));
    m.check(where + QStringLiteral(": without the trims and the gain and delay rows"),
            !m.shown(QStringLiteral("Matrix.row.0.trim")) && !m.shown(QStringLiteral("Matrix.gain.0")) && !m.shown(QStringLiteral("Matrix.delay.0")));
    m.check(where + QStringLiteral(": the keys work there too"), m.focusGrid() && (m.key(Qt::Key_Right), true) && m.mc->currentOutput() == 1);
    m.mc->setCurrent(-1, -1);
    m.check(where + QStringLiteral(": screenshot of the dense grid"), m.d.grab(m.w, shot));
    m.click(denseButton);
    m.check(where + QStringLiteral(": and back to normal"), !m.mc->dense() && m.shown(QStringLiteral("Matrix.gain.0")) != nullptr);
}

// ---- The inspector (the page) ------------------------------------------------------------

void inspector(Mx &m) {
    m.clean();
    m.cc->setRoute(0, 0, true, -3, false);
    m.d.waitIdle();
    m.sc->select(-1);
    m.mc->setCurrent(-1, -1);
    m.settle();
    QQuickItem *summary = m.shown(QStringLiteral("RoutingInspector.summary"));
    m.check(QStringLiteral("page: with nothing chosen the inspector sums the routing up (%1)").arg(summary ? summary->property("text").toString() : QString()),
            m.shown(QStringLiteral("RoutingInspector.none")) && summary && summary->property("text").toString().contains(QLatin1String("crosspoint")));
    m.check(QStringLiteral("page: screenshot, nothing chosen"), m.d.grab(m.w, QStringLiteral("mx1-page.png")));

    m.click(QStringLiteral("Matrix.cell.0.0"));
    m.check(QStringLiteral("page: a cell shows the crosspoint"), m.at(0, 0) && m.shown(QStringLiteral("RoutingInspector.title"))
            && m.shown(QStringLiteral("RoutingInspector.crosspoint")) && !m.shown(QStringLiteral("RoutingInspector.none"))
            && !m.shown(QStringLiteral("ChannelInspector.title")));
    QQuickItem *gain = m.shown(QStringLiteral("RoutingInspector.gain"));
    m.check(QStringLiteral("page: with its gain"), gain && gain->property("value").toDouble() == -3);
    QQuickItem *chips = m.shown(QStringLiteral("RoutingInspector.alsoFeeds.chips"));
    m.check(QStringLiteral("page: and what else the input feeds, as chips"), chips != nullptr);
    if (chips) {
        QQuickItem *chip = m.shown(QStringLiteral("RoutingInspector.alsoFeeds.chips.chip.1"));
        if (chip) {
            QTest::mouseClick(m.w, Qt::LeftButton, Qt::NoModifier, Mx::centre(chip));
            m.settle();
        }
        m.check(QStringLiteral("page: a chip connects the input to that output"), m.connected(0, 2));
    }
    QQuickItem *invert = m.shown(QStringLiteral("RoutingInspector.invert"));
    if (invert) {
        QTest::mouseClick(m.w, Qt::LeftButton, Qt::NoModifier, Mx::centre(invert));
        m.settle();
    }
    m.check(QStringLiteral("page: its Invert phase switch inverts it"), m.route(0, 0, true, -3, true));
    invert = m.shown(QStringLiteral("RoutingInspector.invert"));
    m.check(QStringLiteral("page: and shows it inverted"), invert && invert->property("checked").toBool());
    m.mc->setInvert(0, 0, false);
    m.d.waitIdle();
    m.settle();
    m.check(QStringLiteral("page: and follows the grid back"), invert && !invert->property("checked").toBool());
    m.mc->setInvert(0, 0, true);
    m.d.waitIdle();
    m.settle();
    m.check(QStringLiteral("page: screenshot, a crosspoint"), m.d.grab(m.w, QStringLiteral("mx1-page-crosspoint.png")));

    m.check(QStringLiteral("page: an output's header shows the output"),
            m.click(QStringLiteral("Matrix.header.1.select")) && m.at(-1, -1) && m.shown(QStringLiteral("ChannelInspector.title"))
                && m.shown(QStringLiteral("RoutingInspector.output")) && m.shown(QStringLiteral("RoutingInspector.fedBy")));
    m.check(QStringLiteral("page: with its polarity as two chips"), m.shown(QStringLiteral("RoutingInspector.polarity")) != nullptr);
    QQuickItem *src = m.shown(QStringLiteral("RoutingInspector.fedBy.chips.chip.0"));
    if (src) {
        QTest::mouseClick(m.w, Qt::LeftButton, Qt::NoModifier, Mx::centre(src));
        m.settle();
    }
    m.check(QStringLiteral("page: a Fed by chip connects that input"), m.connected(0, 1));
    m.check(QStringLiteral("page: screenshot, an output"), m.d.grab(m.w, QStringLiteral("mx1-page-output.png")));
    m.check(QStringLiteral("page: an input's row shows the input with its trim"),
            m.click(QStringLiteral("Matrix.row.2.select")) && m.shown(QStringLiteral("RoutingInspector.input"))
                && m.shown(QStringLiteral("RoutingInspector.trim")) && !m.shown(QStringLiteral("RoutingInspector.output")));
    m.check(QStringLiteral("page: screenshot, an input"), m.d.grab(m.w, QStringLiteral("mx1-page-input.png")));
    m.click(QStringLiteral("Matrix.cell.0.0"));
    m.check(QStringLiteral("page: a cell after a row shows the crosspoint again"), m.shown(QStringLiteral("RoutingInspector.crosspoint")) != nullptr);
    m.sc->select(-1);
    m.mc->setCurrent(-1, -1);
    m.clean();
}

// ---- The section -------------------------------------------------------------------------

void mx1Section(SmokeDriver &d) {
    d.allowSectionTime(240);
    Mx m{ d, nullptr, Registrar::instance<MatrixController>(), Registrar::instance<ChannelController>(),
          Registrar::instance<SidebarController>(), d.app().device };
    auto *layout = Registrar::instance<LayoutController>();
    Shell *shell = d.app().shell;
    m.check(QStringLiteral("the controllers"), m.mc && m.cc && m.sc && layout && shell);
    if (!m.mc || !m.cc || !m.sc || !layout || !shell) return;
    if (!m.device->connected() || !m.device->demo() || m.device->readOnly()) m.device->openDemo();
    m.check(QStringLiteral("on the demo"), d.waitFor(10000, [&] { return m.device->connected() && m.device->demo() && !m.device->readOnly(); }) && d.waitIdle());
    for (const QString &action : shell->shownWindows()) shell->closeWindow(action);
    const QString wasLayout = layout->layout();
    QQuickWindow *main = d.mainWindow();
    const QSize wasSize = main->size();
    m.mc->setLinkPairs(false);
    m.mc->setDense(false);

    // ---- Modern: the Routing page
    m.check(QStringLiteral("Modern"), d.switchLayout("modern"));
    main->resize(1600, 1000);
    d.waitFor(1500, [&] { return main->contentItem()->width() == 1600; });
    layout->arrangement()->setProperty("inspector", QStringLiteral("shown"));
    m.w = main;
    m.check(QStringLiteral("the Routing page"), d.goTo(QStringLiteral("routing/matrix"), 300) && m.shown(QStringLiteral("MatrixPanel")));
    m.check(QStringLiteral("page: its tools are in the row over it"),
            m.shown(QStringLiteral("RoutingTools.link")) && m.shown(QStringLiteral("RoutingTools.direct")) && m.shown(QStringLiteral("RoutingTools.clear"))
                && m.shown(QStringLiteral("RoutingTools.dense")) && !m.shown(QStringLiteral("Matrix.tools")));
    drawn(m, QStringLiteral("page"));
    keys(m, QStringLiteral("page"));
    clicks(m, QStringLiteral("page"));
    linked(m, QStringLiteral("page"), QStringLiteral("RoutingTools.link"));
    inspector(m);
    clearQuestion(m, QStringLiteral("page"), QStringLiteral("RoutingTools"));
    dense(m, QStringLiteral("page"), QStringLiteral("RoutingTools.dense"), QStringLiteral("mx1-page-dense.png"));
    // Narrow: the grid scrolls under its headers
    main->resize(860, 540);
    d.waitFor(1500, [&] { return main->contentItem()->width() == 860; });
    m.settle();
    sticky(m, QStringLiteral("page"));
    main->resize(wasSize);
    m.settle();

    // ---- Classic: the window shows the same panel
    m.check(QStringLiteral("Classic"), d.switchLayout("classic"));
    shell->trigger(QStringLiteral("tools.matrix"));
    auto *w = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("tools.matrix")));
    m.check(QStringLiteral("window: Tools > Matrix Mixer opens it"), w && d.waitFor(3000, [&] { return w->isVisible(); }));
    if (w) {
        m.w = w;
        d.waitIdle();
        d.spin(200);
        m.check(QStringLiteral("window: the tools are over the grid"),
                m.shown(QStringLiteral("Matrix.tools")) && m.shown(QStringLiteral("Matrix.link")) && m.shown(QStringLiteral("Matrix.direct"))
                    && m.shown(QStringLiteral("Matrix.clear")) && m.shown(QStringLiteral("Matrix.dense")));
        m.check(QStringLiteral("window: no selection in a window"), !m.shown(QStringLiteral("Matrix.header.1.select")));
        drawn(m, QStringLiteral("window"));
        keys(m, QStringLiteral("window"));
        linked(m, QStringLiteral("window"), QStringLiteral("Matrix.link"));
        clearQuestion(m, QStringLiteral("window"), QStringLiteral("Matrix"));
        m.check(QStringLiteral("window: screenshot"), d.grab(w, QStringLiteral("mx1-window.png")));
        dense(m, QStringLiteral("window"), QStringLiteral("Matrix.dense"), QStringLiteral("mx1-window-dense.png"));
        w->resize(560, 380);
        d.spin(200);
        sticky(m, QStringLiteral("window"));
        w->close();
        d.waitFor(2000, [&] { return !w->isVisible(); });
    }

    // Back as it was
    m.mc->setLinkPairs(false);
    m.mc->setDense(false);
    m.mc->setCurrent(-1, -1);
    m.sc->select(-1);
    d.switchLayout(wasLayout.toLatin1().constData());
    main->resize(wasSize);
    m.device->openDemo();
    m.check(QStringLiteral("the demo reopens"), d.waitFor(5000, [&] { return m.device->connected() && !m.device->readOnly(); }));
    d.waitIdle();
}

const SmokeDriver::Section mx1("mx1-matrix", 897, &mx1Section);

} // namespace
