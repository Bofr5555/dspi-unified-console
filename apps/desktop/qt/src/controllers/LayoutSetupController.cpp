// -- WP UB1 -- (UXP B1, owner D5) Crossovers and routes from the confirmed
// speaker layout (LayoutSetupController.h)
#include "LayoutSetupController.h"

#include "ChannelController.h"
#include "ChannelListModel.h"
#include "CoreApi.h"
#include "DeviceModel.h"
#include "LimitsController.h"
#include "MatrixController.h"
#include "NoticeCenter.h"
#include "SpeakersController.h"

#include <QJsonArray>
#include <QRegularExpression>
#include <QSet>
#include <algorithm>
#include <cmath>
#include <map>
#include <vector>

namespace {

const Registrar::Controller<LayoutSetupController> layoutSetupController;

// The tweeter rule (preflight.rs HIGH_MIN_HZ, MID_MIN_HZ, MIN_SLOPE_DB)
constexpr double kHighMinHz = 1000;
constexpr double kMidMinHz = 80;
constexpr int kMinSlopeDb = 12;
// A crossover's frequency, Hz (the fields' range)
constexpr double kMinHz = 10;
constexpr double kMaxHz = 20000;
// Linkwitz-Riley 24 dB/oct, the sheet's slope until the outputs have one
constexpr int kLinkwitzRiley = 1;
constexpr int kDefaultOrder = 4;
// A stereo input summed into one output: each half at −6 dB
constexpr float kSumDb = -6.0f;

// The ways of a speaker from the lowest up; a full-range output is no way
int rank(const QString &role) {
    if (role == QLatin1String("sub")) return 0;
    if (role == QLatin1String("low")) return 1;
    if (role == QLatin1String("mid")) return 2;
    if (role == QLatin1String("high")) return 3;
    return -1;
}

QString wayWord(const QString &role) {
    if (role == QLatin1String("sub")) return LayoutSetupController::tr("sub");
    if (role == QLatin1String("low")) return LayoutSetupController::tr("woofer");
    if (role == QLatin1String("mid")) return LayoutSetupController::tr("mid driver");
    if (role == QLatin1String("high")) return LayoutSetupController::tr("tweeter");
    return LayoutSetupController::tr("full range");
}

// Where a transition starts when the outputs say nothing
double defaultHz(const QString &lower, const QString &upper) {
    if (lower == QLatin1String("sub") || upper.isEmpty()) return 80;
    if (lower == QLatin1String("low") && upper == QLatin1String("mid")) return 400;
    if (lower == QLatin1String("mid")) return 3000;
    return 2000;
}

// "1 kHz", "1.5 kHz", "80 Hz"
QString hzText(double hz) {
    if (hz >= 1000) return LayoutSetupController::tr("%1 kHz").arg(QString::number(hz / 1000.0, 'g', 3));
    return LayoutSetupController::tr("%1 Hz").arg(std::lround(hz));
}

bool isCrossoverType(int t) { return t >= 32 && t <= 63; }

struct Band {
    int type = 0;
    double freq = 0;
    bool bypass = false;
};

// What output `o`'s crossover bands are (the device's, not a staged edit)
std::vector<Band> bandsOf(const DspState *s, int channel) {
    std::vector<Band> out;
    if (!s || channel < 0 || channel >= MAX_CHANNELS) return out;
    for (int b = 0; b < MAX_XOVER_BANDS; ++b)
        out.push_back({ s->xover[channel][b].filter_type, double(s->xover[channel][b].freq), s->xover[channel][b].bypass });
    return out;
}

// The first active pass of a kind on a channel: its band, or -1
int activePass(const std::vector<Band> &bands, const ChannelController *cc, bool highPass) {
    for (int b = 0; b < int(bands.size()); ++b)
        if (isCrossoverType(bands[b].type) && !bands[b].bypass && cc->isLowPass(bands[b].type) != highPass) return b;
    return -1;
}

struct Write {
    uint8_t channel;
    uint8_t band;
    FilterParams params;
};
struct RouteWrite {
    uint8_t input;
    uint8_t output;
    bool enabled;
    float gain;
    bool invert;
};

} // namespace

LayoutSetupController::LayoutSetupController(const AppContext &ctx, QObject *parent) : QObject(parent), m_ctx(ctx) {
    connect(ctx.device, &DeviceModel::snapshotApplied, this, &LayoutSetupController::bump);
}

void LayoutSetupController::bump() {
    // The other controllers are made in no set order: hooked on first use
    if (!m_hooked) {
        auto *speakers = Registrar::instance<SpeakersController>();
        auto *limits = Registrar::instance<LimitsController>();
        if (speakers && limits) {
            m_hooked = true;
            connect(speakers, &SpeakersController::storedChanged, this, &LayoutSetupController::bump);
            connect(limits, &LimitsController::changed, this, &LayoutSetupController::bump);
            connect(limits, &LimitsController::lockChanged, this, &LayoutSetupController::bump);
        }
    }
    ++m_revision;
    emit changed();
}

bool LayoutSetupController::writable() const {
    const auto *limits = Registrar::instance<LimitsController>();
    return m_ctx.device->connected() && !m_ctx.device->readOnly() && !(limits && limits->locked());
}

QJsonObject LayoutSetupController::layout() const {
    const auto *speakers = Registrar::instance<SpeakersController>();
    if (!speakers || speakers->status() != QLatin1String("confirmed")) return {};
    return speakers->stored();
}

LayoutSetupController::Floor LayoutSetupController::floorOf(const QString &role, double profileHz, int profileSlopeDb) {
    // tweeter_rule then tweeter_floor: a profile may raise a tweeter's, and
    // set a mid driver's; the other ways have the profile's alone
    if (role == QLatin1String("high"))
        return { std::max(kHighMinHz, profileHz > 0 ? profileHz : kHighMinHz), std::max(kMinSlopeDb, profileSlopeDb > 0 ? profileSlopeDb : kMinSlopeDb) };
    if (role == QLatin1String("mid"))
        return { profileHz > 0 ? profileHz : kMidMinHz, profileSlopeDb > 0 ? profileSlopeDb : kMinSlopeDb };
    return { std::max(0.0, profileHz), std::max(0, profileSlopeDb) };
}

LayoutSetupController::Floor LayoutSetupController::floorFor(const QString &role, int output) const {
    double hz = 0;
    int slope = 0;
    if (const auto *limits = Registrar::instance<LimitsController>())
        for (const QVariant &v : limits->outputs()) {
            const QVariantMap r = v.toMap();
            if (r.value(QStringLiteral("output")).toInt() != output) continue;
            hz = r.value(QStringLiteral("highpassMinHz")).toDouble();
            slope = int(std::lround(r.value(QStringLiteral("highpassMinSlopeDb")).toDouble()));
        }
    return floorOf(role, hz, slope);
}

QString LayoutSetupController::sideOf(const QString &name) {
    static const QRegularExpression split(QStringLiteral("[^a-z]+"));
    const QStringList words = name.toLower().split(split, Qt::SkipEmptyParts);
    const auto has = [&](std::initializer_list<const char *> list) {
        for (const char *w : list)
            if (words.contains(QLatin1String(w))) return true;
        return false;
    };
    if (has({ "sub", "subs", "subwoofer", "lfe" })) return QStringLiteral("LFE");
    if (has({ "c", "fc", "centre", "center" })) return QStringLiteral("C");
    if (has({ "sl", "ls" })) return QStringLiteral("SL");
    if (has({ "sr", "rs" })) return QStringLiteral("SR");
    if (has({ "bl", "rl", "sbl" })) return QStringLiteral("BL");
    if (has({ "br", "rr", "sbr" })) return QStringLiteral("BR");
    const bool left = has({ "l", "left", "fl" }), right = has({ "r", "right", "fr" });
    if (left == right) return {};
    const QString side = left ? QStringLiteral("L") : QStringLiteral("R");
    if (has({ "back", "rear" })) return QStringLiteral("B") + side;
    if (has({ "surround", "side" })) return QStringLiteral("S") + side;
    return side;
}

QString LayoutSetupController::outputTitle(int output) const {
    // ChannelInfo.title's rule: the name, then its number unless it says it
    ChannelListModel *rows = m_ctx.device->channels();
    const auto *cc = Registrar::instance<ChannelController>();
    const int row = rows && cc ? rows->rowOfChannel(cc->outputChannel(output)) : -1;
    QString name = row >= 0 ? rows->get(row, QStringLiteral("displayName")).toString() : QString();
    if (name.isEmpty() && row >= 0) name = rows->get(row, QStringLiteral("label")).toString();
    if (name.isEmpty()) return tr("Out %1").arg(output + 1);
    static const QRegularExpression own(QStringLiteral("^(?:Out )?(\\d+)(?!\\d)"));
    const QRegularExpressionMatch m = own.match(name);
    if (m.hasMatch() && m.captured(1) == QString::number(output + 1)) return name;
    //: An output's name and its number, "Left woofer · 1"
    return tr("%1 · %2").arg(name).arg(output + 1);
}

QString LayoutSetupController::inputName(int input) const {
    ChannelListModel *rows = m_ctx.device->channels();
    const int row = rows ? rows->rowOfChannel(input) : -1;
    QString name = row >= 0 ? rows->get(row, QStringLiteral("displayName")).toString() : QString();
    if (name.isEmpty() && row >= 0) name = rows->get(row, QStringLiteral("label")).toString();
    return name.isEmpty() ? tr("In %1").arg(input + 1) : name;
}

// ---- The crossovers ----------------------------------------------------------

QVariantList LayoutSetupController::speakers(const QJsonObject &layout, QStringList *untouched) const {
    const auto *cc = Registrar::instance<ChannelController>();
    const DspState *s = m_ctx.device->state();
    if (!cc || !s) return {};
    // The layout's groups in the order of their first output
    struct Group {
        int group = 0;
        QString speaker;
        std::map<int, std::pair<QString, QVariantList>> ways;   // rank -> role, outputs
    };
    std::vector<Group> groups;
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray()) {
        const QJsonObject r = v.toObject();
        const int o = r.value(QLatin1String("output")).toInt(-1);
        const QString role = r.value(QLatin1String("role")).toString();
        if (o < 0 || o >= cc->numOutputs()) continue;
        if (rank(role) < 0) {
            //: An output the speaker layout calls full range: the sheet leaves its filters alone
            untouched->append(tr("%1 is full range: its filters stay as they are.").arg(outputTitle(o)));
            continue;
        }
        const int g = r.value(QLatin1String("group")).toInt();
        auto it = std::find_if(groups.begin(), groups.end(), [g](const Group &x) { return x.group == g; });
        if (it == groups.end()) {
            groups.push_back({ g, r.value(QLatin1String("speaker")).toString().trimmed(), {} });
            it = groups.end() - 1;
        }
        auto &way = it->ways[rank(role)];
        way.first = role;
        way.second.append(o);
    }

    QVariantList rows;
    QList<QStringList> shapes;   // each row's roles, for "same as"
    for (const Group &g : groups) {
        QStringList roles;
        std::vector<std::pair<QString, QVariantList>> ways;
        for (const auto &[r, way] : g.ways) {
            roles << way.first;
            ways.push_back(way);
        }
        // A lone way: a sub gets its low pass, a woofer or a tweeter alone has
        // no partner to cross over with
        if (ways.size() == 1 && ways.front().first != QLatin1String("sub")) {
            for (const QVariant &o : ways.front().second)
                untouched->append(tr("%1 has no other way in its speaker: its filters stay as they are.").arg(outputTitle(o.toInt())));
            continue;
        }
        QVariantList transitions;
        const size_t n = ways.size() == 1 ? 1 : ways.size() - 1;
        for (size_t i = 0; i < n; ++i) {
            const auto &lower = ways[i];
            const bool alone = ways.size() == 1;
            const QString upperRole = alone ? QString() : ways[i + 1].first;
            const QVariantList upperOutputs = alone ? QVariantList() : ways[i + 1].second;
            // Where the outputs are: the upper way's high pass, else the lower's low pass
            double hz = 0;
            int family = kLinkwitzRiley, order = kDefaultOrder;
            const auto take = [&](const QVariantList &outs, bool highPass) {
                for (const QVariant &o : outs) {
                    const std::vector<Band> bands = bandsOf(s, cc->outputChannel(o.toInt()));
                    const int b = activePass(bands, cc, highPass);
                    if (b < 0) continue;
                    hz = bands[b].freq;
                    family = cc->familyOf(bands[b].type);
                    order = cc->orderOf(bands[b].type);
                    return true;
                }
                return false;
            };
            if (!take(upperOutputs, true)) take(lower.second, false);
            if (family < 1 || order < 1) {
                family = kLinkwitzRiley;
                order = kDefaultOrder;
            }
            if (!(hz > 0)) hz = defaultHz(lower.first, upperRole);
            // The floor: the highest of the upper way's outputs'
            Floor floor;
            for (const QVariant &o : upperOutputs) {
                const Floor f = floorFor(upperRole, o.toInt());
                floor.hz = std::max(floor.hz, f.hz);
                floor.slopeDb = std::max(floor.slopeDb, f.slopeDb);
            }
            hz = std::clamp(std::max(hz, floor.hz), kMinHz, kMaxHz);
            while (order * 6 < floor.slopeDb && order < 8) order = family == 2 ? order + 1 : order + 2;
            //: A crossover between two ways of a speaker, "woofer ◂▸ tweeter"
            const QString words = alone ? tr("low pass") : tr("%1 ◂▸ %2").arg(wayWord(lower.first), wayWord(upperRole));
            transitions.append(QVariantMap{ { QStringLiteral("lowerRole"), lower.first },
                                            { QStringLiteral("upperRole"), upperRole },
                                            { QStringLiteral("words"), words },
                                            { QStringLiteral("hz"), std::round(hz) },
                                            { QStringLiteral("family"), family },
                                            { QStringLiteral("order"), order },
                                            { QStringLiteral("floorHz"), floor.hz },
                                            { QStringLiteral("minSlopeDb"), floor.slopeDb },
                                            { QStringLiteral("lowerOutputs"), lower.second },
                                            { QStringLiteral("upperOutputs"), upperOutputs } });
        }
        QVariantList outputs;
        for (const auto &w : ways) outputs.append(w.second);
        const QString side = sideOf(g.speaker);
        QString title = g.speaker;
        if (side == QLatin1String("L") && g.speaker.size() <= 1) title = tr("Left");
        else if (side == QLatin1String("R") && g.speaker.size() <= 1) title = tr("Right");
        else if (side == QLatin1String("C") && g.speaker.size() <= 1) title = tr("Centre");
        if (title.isEmpty()) title = tr("Speaker %1").arg(g.group + 1);
        rows.append(QVariantMap{ { QStringLiteral("title"), title },
                                 { QStringLiteral("sameAs"), int(shapes.indexOf(roles)) },
                                 { QStringLiteral("outputs"), outputs },
                                 { QStringLiteral("transitions"), transitions } });
        shapes << roles;
    }
    return rows;
}

// ---- The routes ----------------------------------------------------------------

QVariantList LayoutSetupController::routePlan(const QJsonObject &layout, QStringList *notes, QVariantList *lines) const {
    const auto *cc = Registrar::instance<ChannelController>();
    const DspState *s = m_ctx.device->state();
    if (!cc || !s) return {};
    const int inputs = cc->numInputs();
    if (inputs < 1) return {};
    // Each input's side by its name; L and R by place when no name says it
    QHash<QString, int> byName;
    for (int i = 0; i < inputs; ++i) {
        const QString side = sideOf(inputName(i));
        if (!side.isEmpty() && !byName.contains(side)) byName.insert(side, i);
    }
    if (!byName.contains(QStringLiteral("L"))) byName.insert(QStringLiteral("L"), 0);
    if (!byName.contains(QStringLiteral("R"))) byName.insert(QStringLiteral("R"), inputs > 1 ? 1 : 0);
    const int left = byName.value(QStringLiteral("L")), right = byName.value(QStringLiteral("R"));
    // [(input, gain)] for a speaker's side
    const auto sources = [&](const QString &side) -> QList<QPair<int, float>> {
        if (byName.contains(side)) return { { byName.value(side), 0.0f } };
        if (side == QLatin1String("C") || side == QLatin1String("LFE"))
            return left == right ? QList<QPair<int, float>>{ { left, 0.0f } } : QList<QPair<int, float>>{ { left, kSumDb }, { right, kSumDb } };
        // A surround with no input of its own: the back one, else the front
        const QChar lr = side.back();
        const QString other = (side.front() == QLatin1Char('S') ? QStringLiteral("B") : QStringLiteral("S")) + lr;
        if (byName.contains(other)) return { { byName.value(other), 0.0f } };
        return { { lr == QLatin1Char('L') ? left : right, 0.0f } };
    };

    QVariantList routed;
    std::map<int, QStringList> feeds;   // input -> the titles it feeds
    for (const QJsonValue &v : layout.value(QLatin1String("outputs")).toArray()) {
        const QJsonObject r = v.toObject();
        const int o = r.value(QLatin1String("output")).toInt(-1);
        if (o < 0 || o >= cc->numOutputs() || o >= MAX_OUTPUTS) continue;
        QString side = sideOf(r.value(QLatin1String("speaker")).toString());
        if (r.value(QLatin1String("role")).toString() == QLatin1String("sub")) side = QStringLiteral("LFE");
        if (side.isEmpty()) {
            //: An output whose speaker's name ("Desk") says no side: Routing leaves it alone
            notes->append(tr("%1: its speaker's name says no side, so its routes stay as they are.").arg(outputTitle(o)));
            continue;
        }
        QVariantList from;
        for (const auto &[input, gain] : sources(side)) {
            from.append(QVariantMap{ { QStringLiteral("input"), input }, { QStringLiteral("gainDb"), gain } });
            feeds[input] << outputTitle(o);
        }
        routed.append(QVariantMap{ { QStringLiteral("output"), o }, { QStringLiteral("from"), from } });
        if (!s->output_enabled[o])
            //: An output the routing reaches that is switched off
            notes->append(tr("%1 is off: it plays nothing until you turn it on in Delay & Gain.").arg(outputTitle(o)));
    }
    for (const auto &[input, titles] : feeds)
        lines->append(QVariantMap{ { QStringLiteral("input"), input }, { QStringLiteral("name"), inputName(input) },
                                   { QStringLiteral("outputs"), titles } });
    return routed;
}

QVariantMap LayoutSetupController::plan() const {
    const auto *cc = Registrar::instance<ChannelController>();
    const auto *speakers = Registrar::instance<SpeakersController>();
    const auto *limits = Registrar::instance<LimitsController>();
    QVariantMap out{ { QStringLiteral("crossovers"), cc && cc->crossoverSupported() } };
    QString blocker, kind;
    if (!m_ctx.device->connected() || !cc || !speakers) {
        blocker = tr("Open a DSPi first.");
        kind = QStringLiteral("device");
    } else if (m_ctx.device->readOnly()) {
        blocker = tr("This connection may not change the DSPi.");
        kind = QStringLiteral("device");
    } else if (limits && limits->locked()) {
        blocker = tr("The installer's lock is on: unlock it to change crossovers and routing.");
        kind = QStringLiteral("device");
    } else if (speakers->status() != QLatin1String("confirmed")) {
        blocker = tr("Confirm the speaker layout first: which output drives which way of which speaker.");
        kind = QStringLiteral("layout");
    }
    out.insert(QStringLiteral("blocker"), blocker);
    out.insert(QStringLiteral("blockerKind"), kind);
    const QJsonObject l = blocker.isEmpty() ? layout() : QJsonObject();
    QStringList untouched, notes;
    QVariantList lines;
    const QVariantList rows = l.isEmpty() || !cc->crossoverSupported() ? QVariantList() : speakers(l, &untouched);
    const QVariantList routed = l.isEmpty() ? QVariantList() : routePlan(l, &notes, &lines);
    out.insert(QStringLiteral("speakers"), rows);
    out.insert(QStringLiteral("untouched"), untouched);
    out.insert(QStringLiteral("routes"), lines);
    out.insert(QStringLiteral("routeNotes"), notes);
    out.insert(QStringLiteral("routed"), routed);
    // The floor in one line: the tweeters' (1 kHz, or the limits' when higher)
    double tweeter = 0;
    bool limited = false;
    for (const QVariant &row : rows)
        for (const QVariant &t : row.toMap().value(QStringLiteral("transitions")).toList()) {
            const QVariantMap m = t.toMap();
            if (m.value(QStringLiteral("upperRole")).toString() != QLatin1String("high")) continue;
            tweeter = std::max(tweeter, m.value(QStringLiteral("floorHz")).toDouble());
            limited = limited || m.value(QStringLiteral("floorHz")).toDouble() > kHighMinHz;
        }
    out.insert(QStringLiteral("floorText"),
               !(tweeter > 0) ? QString()
               : limited      ? tr("Tweeter high-passes never below %1 (the installer's limits).").arg(hzText(tweeter))
                              : tr("Tweeter high-passes never below %1.").arg(hzText(tweeter)));
    return out;
}

QString LayoutSetupController::check(const QVariantList &values) const {
    const QVariantMap p = plan();
    if (!p.value(QStringLiteral("blocker")).toString().isEmpty()) return p.value(QStringLiteral("blocker")).toString();
    const auto *cc = Registrar::instance<ChannelController>();
    const QVariantList rows = p.value(QStringLiteral("speakers")).toList();
    if (values.size() != rows.size()) return tr("The speaker layout changed: open this again.");
    for (int r = 0; r < rows.size(); ++r) {
        const QVariantMap row = rows[r].toMap();
        const QVariantList ts = row.value(QStringLiteral("transitions")).toList();
        const QVariantList vs = values[r].toMap().value(QStringLiteral("transitions")).toList();
        if (vs.size() != ts.size()) return tr("The speaker layout changed: open this again.");
        for (int i = 0; i < ts.size(); ++i) {
            const QVariantMap t = ts[i].toMap(), v = vs[i].toMap();
            const QString title = row.value(QStringLiteral("title")).toString();
            const double hz = v.value(QStringLiteral("hz")).toDouble();
            const int family = v.value(QStringLiteral("family")).toInt(), order = v.value(QStringLiteral("order")).toInt();
            if (!std::isfinite(hz) || hz < kMinHz || hz > kMaxHz)
                return tr("%1: a crossover between %2 and %3.").arg(title, hzText(kMinHz), hzText(kMaxHz));
            if (cc->crossoverType(family, order, false) < 0) return tr("%1: choose a slope.").arg(title);
            const QString upper = t.value(QStringLiteral("upperRole")).toString();
            const double floorHz = t.value(QStringLiteral("floorHz")).toDouble();
            const int minSlope = t.value(QStringLiteral("minSlopeDb")).toInt();
            const QString way = wayWord(upper);
            if (hz < floorHz - 0.5)
                //: %1 the speaker ("Left"), %2 its way ("tweeter"), %3 the lowest it may go ("1 kHz")
                return tr("%1: a %2's high pass never goes below %3.").arg(title, way, hzText(floorHz));
            if (order * 6 < minSlope)
                //: %1 the speaker, %2 its way, %3 dB/oct
                return tr("%1: a %2's high pass needs %3 dB/oct or steeper.").arg(title, way).arg(minSlope);
        }
    }
    return {};
}

QString LayoutSetupController::apply(const QVariantList &values, bool crossovers, bool routing) {
    if (!crossovers && !routing) return tr("Choose crossovers, routing or both.");
    const QString refusal = crossovers ? check(values) : plan().value(QStringLiteral("blocker")).toString();
    if (!refusal.isEmpty()) return refusal;
    auto *cc = Registrar::instance<ChannelController>();
    const DspState *s = m_ctx.device->state();
    if (!cc || !s) return tr("Open a DSPi first.");
    const QVariantMap p = plan();

    // Each output's high and low pass, then the band each goes to
    std::vector<Write> writes;
    if (crossovers && cc->crossoverSupported()) {
        struct Pass {
            double hz = 0;
            int type = -1;
        };
        std::map<int, std::pair<Pass, Pass>> passes;   // output -> high, low
        const QVariantList rows = p.value(QStringLiteral("speakers")).toList();
        for (int r = 0; r < rows.size(); ++r) {
            const QVariantList ts = rows[r].toMap().value(QStringLiteral("transitions")).toList();
            const QVariantList vs = values[r].toMap().value(QStringLiteral("transitions")).toList();
            for (int i = 0; i < ts.size(); ++i) {
                const QVariantMap t = ts[i].toMap(), v = vs[i].toMap();
                const double hz = v.value(QStringLiteral("hz")).toDouble();
                const int family = v.value(QStringLiteral("family")).toInt(), order = v.value(QStringLiteral("order")).toInt();
                for (const QVariant &o : t.value(QStringLiteral("lowerOutputs")).toList())
                    passes[o.toInt()].second = { hz, cc->crossoverType(family, order, true) };
                for (const QVariant &o : t.value(QStringLiteral("upperOutputs")).toList())
                    passes[o.toInt()].first = { hz, cc->crossoverType(family, order, false) };
            }
        }
        for (const auto &[o, pair] : passes) {
            const int channel = cc->outputChannel(o);
            const std::vector<Band> bands = bandsOf(s, channel);
            QSet<int> taken;
            for (const auto &[pass, highPass] : { std::pair{ pair.first, true }, std::pair{ pair.second, false } }) {
                if (pass.type < 0) continue;
                int at = -1;
                for (int b = 0; b < int(bands.size()) && at < 0; ++b)
                    if (!taken.contains(b) && isCrossoverType(bands[b].type) && cc->isLowPass(bands[b].type) != highPass) at = b;
                for (int b = 0; b < int(bands.size()) && at < 0; ++b)
                    if (!taken.contains(b) && bands[b].type == 0) at = b;
                if (at < 0)
                    return highPass ? tr("%1 has no free crossover band for its high pass.").arg(outputTitle(o))
                                    : tr("%1 has no free crossover band for its low pass.").arg(outputTitle(o));
                taken.insert(at);
                FilterParams f = s->xover[channel][at];
                f.filter_type = uint8_t(pass.type);
                f.freq = float(pass.hz);
                f.q = 0.707f;
                f.gain = 0;
                f.bypass = false;
                writes.push_back({ uint8_t(channel), uint8_t(at), f });
            }
        }
    }

    std::vector<RouteWrite> routes;
    if (routing) {
        const int inputs = cc->numInputs();
        for (const QVariant &v : p.value(QStringLiteral("routed")).toList()) {
            const QVariantMap r = v.toMap();
            const int o = r.value(QStringLiteral("output")).toInt();
            QHash<int, float> want;
            for (const QVariant &f : r.value(QStringLiteral("from")).toList())
                want.insert(f.toMap().value(QStringLiteral("input")).toInt(), f.toMap().value(QStringLiteral("gainDb")).toFloat());
            // A new crosspoint takes the output's polarity
            const bool inverted = cc->outputPolarity(o) == 1;
            for (int i = 0; i < inputs && i < MAX_INPUTS; ++i) {
                const bool on = s->matrix_routing[i][o];
                if (want.contains(i) && !on) routes.push_back({ uint8_t(i), uint8_t(o), true, want.value(i), inverted });
                else if (!want.contains(i) && on) routes.push_back({ uint8_t(i), uint8_t(o), false, s->matrix_gain[i][o], s->matrix_invert[i][o] });
            }
        }
    }
    // The toast says what was done (in both layouts)
    const auto said = [this](const QString &text) {
        if (auto *notices = Registrar::instance<NoticeCenter>()) notices->post(text, QStringLiteral("info"));
        emit applied(text);
    };
    if (writes.empty() && routes.empty()) {
        said(tr("The outputs already are as the speaker layout says."));
        return {};
    }
    if (auto *matrix = Registrar::instance<MatrixController>(); matrix && !routes.empty()) matrix->model()->unstageAll();
    // One undo step: the core's group around the writes (Direct 1:1's way)
    const QByteArray label = tr("From Speaker Layout").toUtf8();
    m_ctx.device->write(QStringLiteral("layout setup"), [writes, routes, label](FfiCore *c) {
        bool ok = true;
        dspi_undo_group_begin(c, label.constData());
        for (const Write &w : writes) ok = dspi_set_crossover(c, w.channel, w.band, w.params) && ok;
        for (const RouteWrite &r : routes) ok = dspi_set_matrix_route(c, r.input, r.output, r.enabled, r.gain, r.invert) && ok;
        dspi_undo_group_end(c);
        return ok;
    });
    said(crossovers && routing ? tr("Crossovers and routing set from the speaker layout. Undo puts them back in one step.")
                 : crossovers          ? tr("Crossovers set from the speaker layout. Undo puts them back in one step.")
                                       : tr("Routing set from the speaker layout. Undo puts it back in one step."));
    return {};
}

// ---- The partner default ---------------------------------------------------------

double LayoutSetupController::partnerFrequency(int channel, bool highPass) const {
    const auto *cc = Registrar::instance<ChannelController>();
    const auto *speakers = Registrar::instance<SpeakersController>();
    const DspState *s = m_ctx.device->state();
    if (!cc || !speakers || !s) return 0;
    const int output = channel - cc->numInputs();
    if (output < 0 || output >= cc->numOutputs()) return 0;
    // The layout as kept, else as proposed: a starting frequency is no write
    const QVariantMap me = speakers->rowOf(output);
    const QString role = me.value(QStringLiteral("role")).toString();
    const int mine = rank(role);
    if (!me.value(QStringLiteral("included")).toBool() || mine < 0) return 0;
    // The nearest way below (for a high pass) or above (a low pass) in the speaker
    int best = -1;
    double hz = 0;
    for (int o = 0; o < cc->numOutputs(); ++o) {
        const QVariantMap r = speakers->rowOf(o);
        const int way = rank(r.value(QStringLiteral("role")).toString());
        if (o == output || !r.value(QStringLiteral("included")).toBool() || way < 0
            || r.value(QStringLiteral("group")).toInt() != me.value(QStringLiteral("group")).toInt())
            continue;
        if (highPass ? way >= mine : way <= mine) continue;
        if (best >= 0 && (highPass ? way <= best : way >= best)) continue;
        const std::vector<Band> bands = bandsOf(s, cc->outputChannel(o));
        const int b = activePass(bands, cc, !highPass);
        if (b < 0) continue;
        best = way;
        hz = bands[b].freq;
    }
    if (!(hz > 0)) return 0;
    if (highPass) hz = std::max(hz, floorFor(role, output).hz);
    return std::round(hz);
}
