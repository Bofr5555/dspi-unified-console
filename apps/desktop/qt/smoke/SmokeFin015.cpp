// -- WP FIN015 -- (REVF D1) One channel, one name on a page, on the demo with
// output 1 the centre speaker's woofer in the speaker layout ("Centre woofer"):
// - "fin015-names": Equaliser › Delay & Gain's row and the Crossover table's
//   row say "Centre woofer" as the page's list and header do; the Spectrum
//   chooser names it so; Settings › Speakers says "Out 1 (Centre woofer)";
//   Events show the core's "Out 1" as "Out 1 (Centre woofer)", while the
//   record (what an agent reads) keeps the core's words.
#include "SmokeDriver.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "Core.h"
#include "CoreApi.h"
#include "CoreWorker.h"
#include "DeviceModel.h"
#include "EventsController.h"
#include "Registrar.h"
#include "RtaController.h"
#include "SettingsController.h"
#include "Snapshot.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQuickItem>
#include <QQuickWindow>
#include <QRegularExpression>
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

// The texts of the Texts under `from`
QStringList textsUnder(QQuickItem *from) {
    QStringList out;
    if (!from) return out;
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

// A core call on the active device's worker, waited for (SmokeInte's)
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

void namesSection(SmokeDriver &d) {
    DeviceModel *device = d.app().device;
    auto *cc = Registrar::instance<ChannelController>();
    auto *rta = Registrar::instance<RtaController>();
    auto *settings = Registrar::instance<SettingsController>();
    auto *events = Registrar::instance<EventsController>();
    QQuickWindow *w = d.mainWindow();
    d.check("fin015: the controllers and the window", cc && rta && settings && events && w);
    if (!cc || !rta || !settings || !events || !w) return;
    d.check("fin015: on the demo", d.waitFor(10000, [&] { return device->connected(); }) && d.waitIdle());
    // Output 1 is the centre speaker's woofer in the speaker layout (inte-words' layout)
    const QJsonValue stored = onWorker(d, [](FfiCore *c) {
                                  return fill([c](char *b, uint32_t l) { return dspi_speaker_layout(c, b, l); });
                              }).value(QLatin1String("value"));
    const QJsonObject layout{ { QStringLiteral("outputs"),
                                QJsonArray{ QJsonObject{ { QStringLiteral("output"), 0 },
                                                         { QStringLiteral("role"), QStringLiteral("low") },
                                                         { QStringLiteral("speaker"), QStringLiteral("C") },
                                                         { QStringLiteral("group"), 0 } } } },
                              { QStringLiteral("confirmedByUser"), true } };
    const auto setLayout = [&](const QJsonObject &l) {
        const QByteArray text = QJsonDocument(l).toJson(QJsonDocument::Compact);
        return onWorker(d, [text](FfiCore *c) {
            return fill([c, &text](char *b, uint32_t n) { return dspi_speaker_layout_set(c, text.constData(), b, n); });
        });
    };
    setLayout(layout);
    device->refresh();
    const QString word = QStringLiteral("Centre woofer");
    const int ch = cc->outputChannel(0);
    d.check("fin015: the core names output 1 from the layout", d.waitFor(8000, [&] {
        return device->snapshot().channelWords.value(QLatin1String("fromLayout")).toArray().contains(QJsonValue(0))
            && rowValue(device, ch, "displayName").toString() == word;
    }));
    const QString label = rowValue(device, ch, "label").toString();
    d.check(QStringLiteral("fin015: its name (%1) is not its label (%2)").arg(word, label), !label.isEmpty() && label != word);
    ChannelListModel *rows = device->channels();

    // The words beside the pages: "Out 1 (Centre woofer)", once, never in another number
    d.check("fin015: output 1's name for the core's words", rows->outputName(0) == word);
    const QString line = QStringLiteral("EQ band 3 on Out 1: Peaking 63 Hz");
    d.check(QStringLiteral("fin015: an event's words name it (%1)").arg(rows->withNames(line)),
            rows->withNames(line) == QStringLiteral("EQ band 3 on Out 1 (Centre woofer): Peaking 63 Hz"));
    d.check("fin015: a route's too", rows->withNames(QStringLiteral("Route In 3 → Out 1: on at 0 dB"))
                                         == QStringLiteral("Route In 3 → Out 1 (Centre woofer): on at 0 dB"));
    d.check("fin015: named once", rows->withNames(QStringLiteral("change Out 1 (Centre woofer) gain"))
                                      == QStringLiteral("change Out 1 (Centre woofer) gain"));
    d.check("fin015: Out 10 is not Out 1", !rows->withNames(QStringLiteral("Out 10 gain")).contains(word));

    // The Spectrum chooser
    const QVariantMap info = rta->channelInfo(RTA_TAP_OUTPUT, 0);
    d.check(QStringLiteral("fin015: the Spectrum chooser names it %1 (%2)").arg(word, info.value(QStringLiteral("name")).toString()),
            info.value(QStringLiteral("name")).toString() == word);

    const QSize original = w->size();
    if (d.switchLayout("modern")) {
        w->resize(QSize(1600, 1000));
        // Equaliser › Delay & Gain and Crossover: the row's name is the page's
        d.goTo(QStringLiteral("equaliser/delay"), 300);
        QQuickItem *delayRow = shownItem(w->contentItem(), QStringLiteral("EqualiserDelay.row.0"));
        d.check(QStringLiteral("fin015: Delay & Gain's row names it %1 (%2)").arg(word, textsUnder(delayRow).join(QLatin1Char('|'))),
                delayRow && textsUnder(delayRow).contains(word) && !textsUnder(delayRow).contains(label));
        d.goTo(QStringLiteral("equaliser/crossover"), 300);
        QQuickItem *xoRow = shownItem(w->contentItem(), QStringLiteral("EqualiserCrossover.row.0"));
        d.check(QStringLiteral("fin015: the Crossover table's row names it %1 (%2)").arg(word, textsUnder(xoRow).join(QLatin1Char('|'))),
                xoRow && textsUnder(xoRow).contains(word) && !textsUnder(xoRow).contains(label));

        // Settings › Speakers: the output, and the name the pages give it
        d.goTo(QStringLiteral("settings/device"), 200);
        settings->open(QStringLiteral("speakers"));
        d.waitIdle();
        d.spin(300);
        const QString both = QStringLiteral("%1 (%2)").arg(label, word);
        QQuickItem *included = nullptr;
        d.check(QStringLiteral("fin015: Settings › Speakers says %1").arg(both), d.waitFor(3000, [&] {
            included = shownItem(w->contentItem(), QStringLiteral("Speakers.included.0"));
            return included && included->property("text").toString() == both;
        }));

        // System › Events: a change on output 1, shown with its name; the record keeps the core's words
        d.goTo(QStringLiteral("system/events"), 300);
        const double gain = cc->output(0).value(QStringLiteral("gain")).toDouble();
        cc->setOutputGain(0, gain > -10 ? gain - 1 : gain + 1);
        d.waitIdle();
        cc->setOutputGain(0, gain);
        d.waitIdle();
        static const QRegularExpression out1(QStringLiteral("\\bOut 1(?!\\d)"));
        int at = -1;
        const bool logged = d.waitFor(8000, [&] {
            events->reload();
            d.spin(200);
            const QVariantList entries = events->entries();
            for (int i = 0; i < entries.size(); ++i)
                if (out1.match(entries.at(i).toMap().value(QStringLiteral("text")).toString()).hasMatch()) {
                    at = i;
                    return true;
                }
            return false;
        });
        if (logged) {
            const QString text = events->entries().at(at).toMap().value(QStringLiteral("text")).toString();
            d.check(QStringLiteral("fin015: the record keeps the core's words (%1)").arg(text), !text.contains(word));
            QQuickItem *row = nullptr;
            d.check(QStringLiteral("fin015: Events show it with its name"), d.waitFor(3000, [&] {
                row = shownItem(w->contentItem(), QStringLiteral("Events.row.%1").arg(at));
                return row && row->property("title").toString().contains(QStringLiteral("Out 1 (Centre woofer)"));
            }));
        } else {
            d.notRun("fin015: Events show it with its name", QStringLiteral("no event on Out 1 was logged"));
        }
        w->resize(original);
        d.switchLayout("classic");
    } else {
        d.check("fin015: in Modern", false);
    }

    // The layout as it was
    if (stored.isObject()) {
        setLayout(stored.toObject());
        device->refresh();
        d.waitIdle();
    }
}

const SmokeDriver::Section names("fin015-names", 990, &namesSection);

} // namespace
