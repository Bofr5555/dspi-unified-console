// -- WP UA1 -- (UXP A1, C5) One name per output in Modern, on the demo with a
// stereo two-way speaker layout (outputs 1-4: Left woofer, Left tweeter, Right
// woofer, Right tweeter) and nothing routed to the other outputs:
// - "ua1-names": the Crossover and Delay & Gain rows say "Left woofer" and
//   "· 1"; the unused outputs fold under one row ("5 unused") that opens them
//   as "5 · None"; the matrix's column head, the Routing inspector's title and
//   its "also feeds" chips (the connected outputs only, "+" for the rest) and
//   Settings › Installer's headings name the outputs so; Classic's matrix head
//   keeps the label; ChannelInfo's keywords find "tweeter", "Left" and "2".
//   An unused output the generator plays on is never folded (FIN015's `hidden`).
//   Screenshots of each, dark and light, go to <out>/ua1/.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "MatrixController.h"
#include "NoticeCenter.h"
#include "Registrar.h"
#include "SettingsController.h"
#include "Shell.h"
#include "SidebarController.h"
#include "SiggenController.h"
#include "Snapshot.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlComponent>
#include <QQuickItem>
#include <QQuickWindow>
#include <functional>

namespace {

// The shown item named `name` under `from`
QQuickItem *shownItem(QQuickItem *from, const QString &name) {
    if (!from) return nullptr;
    if (from->objectName() == name && from->isVisible()) return from;
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children)
        if (QQuickItem *found = shownItem(child, name)) return found;
    return nullptr;
}

// The texts of the shown Texts under `from`
QStringList textsUnder(QQuickItem *from) {
    QStringList out;
    if (!from || !from->isVisible()) return out;
    if (from->inherits("QQuickText")) out << from->property("text").toString();
    const QList<QQuickItem *> children = from->childItems();
    for (QQuickItem *child : children) out << textsUnder(child);
    return out;
}

QByteArray fill(const std::function<uint32_t(char *, uint32_t)> &call) {
    QByteArray buf(1 << 16, '\0');
    const uint32_t n = call(buf.data(), uint32_t(buf.size()));
    if (n == 0 || n > uint32_t(buf.size())) return {};
    buf.truncate(int(n) - 1);
    return buf;
}

// A core call on the active device's worker, waited for (SmokeFin015's)
QJsonObject onWorker(SmokeDriver &d, std::function<QByteArray(FfiCore *)> job) {
    QByteArray out;
    bool done = false;
    QObject ctx;
    d.app().core->worker()->call(&ctx, std::move(job), [&](const QByteArray &r) {
        out = r;
        done = true;
    });
    d.waitFor(5000, [&] { return done; });
    return QJsonDocument::fromJson(out).object();
}

QVariant rowValue(DeviceModel *device, int channel, const char *role) {
    ChannelListModel *rows = device->channels();
    const int row = rows ? rows->rowOfChannel(channel) : -1;
    return row >= 0 ? rows->get(row, QString::fromLatin1(role)) : QVariant();
}

struct Route {
    bool on = false;
    double gain = 0;
    bool invert = false;
};

void namesSection(SmokeDriver &d) {
    d.allowSectionTime(180);
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    auto *cc = Registrar::instance<ChannelController>();
    auto *mc = Registrar::instance<MatrixController>();
    auto *sc = Registrar::instance<SidebarController>();
    auto *settings = Registrar::instance<SettingsController>();
    QQuickWindow *w = d.mainWindow();
    d.check("ua1: the controllers and the window", cc && mc && sc && settings && w);
    if (!cc || !mc || !sc || !settings || !w) return;
    d.check("ua1: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    const int inputs = cc->numInputs();
    const int outputs = cc->numOutputs();
    d.check(QStringLiteral("ua1: two inputs and more than four outputs (%1, %2)").arg(inputs).arg(outputs), inputs >= 2 && outputs > 4);
    if (inputs < 2 || outputs <= 4) return;

    // The routes as they are, then L to outputs 1-2, R to 3-4, nothing to the rest
    const auto s = [&] { return device->state(); };
    QVector<QVector<Route>> was(outputs, QVector<Route>(inputs));
    for (int o = 0; o < outputs && s(); ++o)
        for (int i = 0; i < inputs; ++i)
            was[o][i] = { s()->matrix_routing[i][o], double(s()->matrix_gain[i][o]), s()->matrix_invert[i][o] };
    const auto wanted = [](int i, int o) { return (i == 0 && o < 2) || (i == 1 && (o == 2 || o == 3)); };
    for (int o = 0; o < outputs; ++o)
        for (int i = 0; i < inputs; ++i)
            if (was[o][i].on != wanted(i, o)) cc->setRoute(i, o, wanted(i, o), 0, false);
    d.waitIdle();

    // The speaker layout: a stereo two-way pair, the user's
    const QJsonValue stored = onWorker(d, [](FfiCore *c) {
                                  return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); });
                              }).value(QLatin1String("value"));
    const auto row = [](int o, const char *role, const char *speaker, int group) {
        return QJsonObject{ { QStringLiteral("output"), o }, { QStringLiteral("role"), QLatin1String(role) },
                            { QStringLiteral("speaker"), QLatin1String(speaker) }, { QStringLiteral("group"), group },
                            { QStringLiteral("source"), QStringLiteral("user") } };
    };
    const QJsonObject layout{ { QStringLiteral("outputs"),
                                QJsonArray{ row(0, "low", "L", 0), row(1, "high", "L", 0), row(2, "low", "R", 1), row(3, "high", "R", 1) } },
                              { QStringLiteral("confirmedByUser"), true } };
    const auto setLayout = [&](const QJsonObject &l) {
        const QByteArray text = QJsonDocument(l).toJson(QJsonDocument::Compact);
        return onWorker(d, [text](FfiCore *c) {
            return fill([c, &text](char *b, uint32_t n) { return dspi_speaker_layout_set(c, text.constData(), b, n); });
        });
    };
    setLayout(layout);
    device->refresh();
    const int unusedCount = outputs - 4;
    d.check("ua1: the core names outputs 1-4 from the layout, the rest unused", d.waitFor(8000, [&] {
        return rowValue(device, cc->outputChannel(0), "displayName").toString() == QLatin1String("Left woofer")
            && rowValue(device, cc->outputChannel(3), "displayName").toString() == QLatin1String("Right tweeter")
            && rowValue(device, cc->outputChannel(4), "unused").toBool();
    }));
    const QString noneLabel = rowValue(device, cc->outputChannel(4), "label").toString();
    d.check(QStringLiteral("ua1: output 5 is \"5 · None\" (%1)").arg(noneLabel), noneLabel == QStringLiteral("5 · None"));

    // ChannelInfo's keywords, for the command palette
    {
        QQmlComponent c(d.app().engine, QStringLiteral("UConsole"), QStringLiteral("ChannelInfo"));
        QObject *info = c.createWithInitialProperties({ { QStringLiteral("channel"), cc->outputChannel(1) } });
        const QStringList words = info ? info->property("keywords").toStringList() : QStringList();
        d.check(QStringLiteral("ua1: output 2's keywords (%1)").arg(words.join(QLatin1Char('|'))),
                words.contains(QStringLiteral("tweeter")) && words.contains(QStringLiteral("Left"))
                    && words.contains(QStringLiteral("Out 2")) && words.contains(QStringLiteral("2")));
        d.check(QStringLiteral("ua1: output 2's title (%1)").arg(info ? info->property("title").toString() : QString()),
                info && info->property("title").toString() == QStringLiteral("Left tweeter · 2"));
        delete info;
    }

    QDir(d.outDir()).mkpath(QStringLiteral("ua1"));
    const QSize original = w->size();
    const Qt::ColorScheme scheme = shell->colorSchemeOverride();
    if (!d.switchLayout("modern")) {
        d.check("ua1: in Modern", false);
        return;
    }
    if (auto *notices = Registrar::instance<NoticeCenter>()) notices->clear();
    w->resize(QSize(1600, 1000));
    d.waitFor(1000, [&] { return w->contentItem()->width() == 1600; });
    sc->select(cc->outputChannel(0));
    d.waitIdle();
    const auto shoot = [&](const QString &name, const char *schemeName) {
        if (QQuickItem *focused = w->activeFocusItem(); focused && focused != w->contentItem()) focused->setFocus(false);
        d.spin(250);
        d.check(QStringLiteral("ua1: screenshot %1-%2").arg(name, QLatin1String(schemeName)),
                d.grab(w, QStringLiteral("ua1/%1-%2.png").arg(name, QLatin1String(schemeName))));
    };
    const auto press = [&](const QString &name) {
        QQuickItem *it = shownItem(w->contentItem(), name);
        return it && SmokeDriver::press(it) && (d.spin(250), true);
    };
    const auto nameAndNumber = [&](const QString &rowName, const QString &name, const QString &number) {
        const QStringList texts = textsUnder(shownItem(w->contentItem(), rowName));
        d.check(QStringLiteral("ua1: %1 says %2, %3 (%4)").arg(rowName, name, number, texts.join(QLatin1Char('|'))),
                texts.contains(name) && texts.contains(number));
    };

    for (const char *schemeName : { "dark", "light" }) {
        const bool first = QLatin1String(schemeName) == QLatin1String("dark");
        shell->setColorSchemeOverride(first ? Qt::ColorScheme::Dark : Qt::ColorScheme::Light);
        d.spin(300);

        // Equaliser › Crossover and Delay & Gain: name, number; the unused folded
        for (const char *page : { "crossover", "delay" }) {
            const QString key = QStringLiteral("equaliser/%1").arg(QLatin1String(page));
            const QString prefix = QLatin1String(page) == QLatin1String("delay") ? QStringLiteral("EqualiserDelay")
                                                                                   : QStringLiteral("EqualiserCrossover");
            d.goTo(key, 300);
            if (first) {
                nameAndNumber(prefix + QStringLiteral(".row.0"), QStringLiteral("Left woofer"), QStringLiteral("· 1"));
                nameAndNumber(prefix + QStringLiteral(".row.3"), QStringLiteral("Right tweeter"), QStringLiteral("· 4"));
                d.check(QStringLiteral("ua1: %1 folds output 5").arg(key), !shownItem(w->contentItem(), prefix + QStringLiteral(".row.4")));
                QQuickItem *fold = shownItem(w->contentItem(), prefix + QStringLiteral(".unused"));
                const QStringList foldTexts = textsUnder(fold);
                d.check(QStringLiteral("ua1: %1's fold row says \"%2 unused\" (%3)").arg(key).arg(unusedCount).arg(foldTexts.join(QLatin1Char('|'))),
                        foldTexts.contains(QStringLiteral("%1 unused").arg(unusedCount)));
            }
            shoot(QStringLiteral("%1-folded").arg(QLatin1String(page)), schemeName);
            const bool opened = press(prefix + QStringLiteral(".unused"));
            if (first) {
                const QStringList texts = textsUnder(shownItem(w->contentItem(), prefix + QStringLiteral(".row.4")));
                d.check(QStringLiteral("ua1: %1 opened shows output 5 as \"5 · None\" (%2)").arg(key, texts.join(QLatin1Char('|'))),
                        opened && texts.contains(QStringLiteral("5 · None")) && !texts.contains(QStringLiteral("· 5")));
            }
            shoot(QStringLiteral("%1-open").arg(QLatin1String(page)), schemeName);
            if (opened) {
                press(prefix + QStringLiteral(".unused"));
                if (first)
                    d.check(QStringLiteral("ua1: %1 folds again").arg(key), !shownItem(w->contentItem(), prefix + QStringLiteral(".row.4")));
            }
            // A playing output is never folded: quiet noise on output 6 alone
            auto *sg = Registrar::instance<SiggenController>();
            if (first && sg && d.waitFor(5000, [&] { return sg->available(); })) {
                sg->selectType(SiggenController::Pink);
                sg->noOutputs();
                sg->stepOutput(5);
                sg->setLevel(-60);
                sg->play();
                d.check(QStringLiteral("ua1: %1 shows output 6 while the generator plays on it").arg(key), d.waitFor(5000, [&] {
                    return sg->playing() && shownItem(w->contentItem(), prefix + QStringLiteral(".row.5"))
                        && !shownItem(w->contentItem(), prefix + QStringLiteral(".row.4"));
                }));
                const QStringList foldTexts = textsUnder(shownItem(w->contentItem(), prefix + QStringLiteral(".unused")));
                d.check(QStringLiteral("ua1: and folds the other %1 (%2)").arg(unusedCount - 1).arg(foldTexts.join(QLatin1Char('|'))),
                        foldTexts.contains(QStringLiteral("%1 unused").arg(unusedCount - 1)));
                sg->stopPlaying(true);
                d.check(QStringLiteral("ua1: stopped, %1 folds it again").arg(key), d.waitFor(5000, [&] {
                    return !sg->playing() && !shownItem(w->contentItem(), prefix + QStringLiteral(".row.5"));
                }));
            } else if (first) {
                d.check("ua1: the generator", false);
            }
        }

        // Routing: the column head and the inspector on L → Left woofer
        d.goTo(QStringLiteral("routing/matrix"), 300);
        mc->setCurrent(0, 0);
        d.spin(300);
        if (first) {
            QObject *head = d.object(QStringLiteral("Matrix.header.0"));
            d.check(QStringLiteral("ua1: the matrix head names output 1 over its pill (%1)").arg(head ? head->property("headName").toString() : QString()),
                    head && head->property("headName").toString() == QStringLiteral("Left woofer")
                        && head->property("shownName").toString() == QStringLiteral("Left woofer · 1"));
            QObject *five = d.object(QStringLiteral("Matrix.header.4"));
            d.check(QStringLiteral("ua1: and output 5 as \"5 · None\" (%1)").arg(five ? five->property("headName").toString() : QString()),
                    five && five->property("headName").toString() == QStringLiteral("5 · None"));
            QQuickItem *title = shownItem(w->contentItem(), QStringLiteral("RoutingInspector.title.output"));
            d.check(QStringLiteral("ua1: the inspector's title names it (%1)").arg(title ? title->property("text").toString() : QString()),
                    title && title->property("text").toString() == QStringLiteral("Left woofer · 1"));
            QObject *chips = d.object(QStringLiteral("RoutingInspector.alsoFeeds.chips"));
            const QVariantList model = chips ? chips->property("model").toList() : QVariantList();
            d.check(QStringLiteral("ua1: L also feeds one output, Left tweeter · 2 (%1)").arg(model.size()),
                    model.size() == 1 && model.at(0).toMap().value(QStringLiteral("text")).toString() == QStringLiteral("Left tweeter · 2"));
            d.check("ua1: \"+\" offers the others", shownItem(w->contentItem(), QStringLiteral("RoutingInspector.alsoFeeds.add")) != nullptr);
        }
        shoot(QStringLiteral("routing"), schemeName);
        mc->setCurrent(-1, -1);

        // Settings › Installer: the per-output headings
        d.goTo(QStringLiteral("settings/device"), 200);
        settings->open(QStringLiteral("installer"));
        d.waitIdle();
        d.spin(300);
        if (first) {
            QObject *group = d.object(QStringLiteral("Installer.output.0"));
            d.check(QStringLiteral("ua1: the Installer's heading names output 1 (%1)").arg(group ? group->property("title").toString() : QString()),
                    group && group->property("title").toString() == QStringLiteral("Left woofer · 1"));
        }
        shoot(QStringLiteral("installer"), schemeName);
    }
    shell->setColorSchemeOverride(scheme);
    w->resize(original);
    d.goTo(QStringLiteral("overview/overview"));
    d.switchLayout("classic");

    // Classic's matrix window: its head keeps the label
    shell->trigger(QStringLiteral("tools.matrix"));
    auto *matrix = qobject_cast<QQuickWindow *>(shell->openWindow(QStringLiteral("tools.matrix")));
    d.check("ua1: Classic's matrix window", matrix && d.waitFor(3000, [&] { return matrix->isVisible(); }) && d.waitIdle());
    if (matrix) {
        d.spin(200);
        QQuickItem *head = shownItem(matrix->contentItem(), QStringLiteral("Matrix.header.0"));
        const QString label = rowValue(device, cc->outputChannel(0), "label").toString();
        d.check(QStringLiteral("ua1: Classic's matrix head keeps the label %1 (%2)").arg(label, head ? head->property("headName").toString() : QString()),
                head && !label.isEmpty() && head->property("headName").toString() == label && head->property("shownName").toString() == label);
        matrix->close();
        d.spin(100);
    }

    // As it was: the layout, then the routes
    if (stored.isObject()) setLayout(stored.toObject());
    for (int o = 0; o < outputs; ++o)
        for (int i = 0; i < inputs; ++i)
            if (was[o][i].on != wanted(i, o) || was[o][i].on) cc->setRoute(i, o, was[o][i].on, was[o][i].gain, was[o][i].invert);
    device->refresh();
    d.waitIdle();
}

const SmokeDriver::Section names("ua1-names", 991, &namesSection);

} // namespace
