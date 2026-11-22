// -- WP LAY6 -- The command palette: its shortcut and its search
#include "PaletteController.h"

#include "LayoutController.h"
#include "TrayController.h"

#include <QGuiApplication>
#include <QJSEngine>
#include <QStringList>
#include <QWindow>
#include <algorithm>
#include <vector>

static const Registrar::Controller<PaletteController> paletteController;

namespace {

// For comparing: no case, no accents
QString folded(const QString &text) {
    const QString parts = text.normalized(QString::NormalizationForm_KD);
    QString out;
    out.reserve(parts.size());
    for (const QChar c : parts)
        if (c.category() != QChar::Mark_NonSpacing) out += c;
    return out.toCaseFolded();
}

bool startsWord(const QString &text, qsizetype i) {
    return text.at(i).isLetterOrNumber() && (i == 0 || !text.at(i - 1).isLetterOrNumber());
}

// `word` is the first letters of words of `text` that follow each other
bool isInitials(const QString &word, const QString &text) {
    QString initials;
    for (qsizetype i = 0; i < text.size(); ++i)
        if (startsWord(text, i)) initials += text.at(i);
    return initials.contains(word);
}

// How many letters of `text` lie between those of `word`, which are there in
// order from the start of a word on; -1 for too many, or not there
qsizetype lettersBetween(const QString &word, const QString &text) {
    qsizetype best = -1;
    for (qsizetype start = 0; start < text.size(); ++start) {
        if (text.at(start) != word.at(0) || !startsWord(text, start)) continue;
        qsizetype at = start, n = 1;
        while (n < word.size() && ++at < text.size())
            if (text.at(at) == word.at(n)) ++n;
        if (n < word.size()) continue;
        const qsizetype between = at - start + 1 - word.size();
        if (between <= 2 * word.size() && (best < 0 || between < best)) best = between;
    }
    return best;
}

// One word of the query in a title, 0 to 100
double titleScore(const QString &word, const QString &title) {
    if (title.startsWith(word)) return 100;
    double found = 0;
    for (qsizetype i = title.indexOf(word); i >= 0; i = title.indexOf(word, i + 1)) {
        if (startsWord(title, i)) return 90;
        found = 70;
    }
    // A single letter is not looked for inside words
    if (word.size() < 2 && word.at(0).isLetterOrNumber()) return 0;
    if (found > 0) return found;
    if (isInitials(word, title)) return 65;
    if (word.size() < 3) return 0;
    const qsizetype between = lettersBetween(word, title);
    return between < 0 ? 0 : std::max(30.0, 55.0 - 2.0 * double(between));
}

// One word of the query in a keyword
double keywordScore(const QString &word, const QString &keyword) {
    if (keyword == word) return 62;
    for (qsizetype i = keyword.indexOf(word); i >= 0; i = keyword.indexOf(word, i + 1))
        if (startsWord(keyword, i)) return 60;
    return word.size() >= 3 && keyword.contains(word) ? 45 : 0;
}

bool isWord(const QString &text) {
    return std::any_of(text.cbegin(), text.cend(), [](QChar c) { return c.isLetterOrNumber(); });
}

// What is looked for: marks alone ("·") only when there is nothing else
QStringList words(const QString &query) {
    const QStringList all = folded(query).split(QLatin1Char(' '), Qt::SkipEmptyParts);
    QStringList out;
    for (const QString &w : all)
        if (isWord(w)) out.append(w);
    return out.isEmpty() ? all : out;
}

// The window's modal popup asks something (SnapshotsController's dialogOpen)
bool asking(const QWindow *window) {
    const QList<QObject *> all = window->findChildren<QObject *>();
    return std::any_of(all.cbegin(), all.cend(), [](const QObject *o) {
        return o->inherits("QQuickPopup") && o->property("modal").toBool() && o->property("opened").toBool();
    });
}

} // namespace

PaletteController::PaletteController(const AppContext &, QObject *parent) : QObject(parent) {}

PaletteController *PaletteController::create(QQmlEngine *, QJSEngine *) {
    auto *self = Registrar::instance<PaletteController>();
    QJSEngine::setObjectOwnership(self, QJSEngine::CppOwnership);
    return self;
}

void PaletteController::open() {
    const auto *layout = Registrar::instance<LayoutController>();
    if (!layout || !layout->modern() || QGuiApplication::modalWindow()) return;
    QWindow *main = nullptr;
    const QWindowList windows = QGuiApplication::topLevelWindows();
    for (QWindow *w : windows)
        if (w->objectName() == QLatin1String("MainWindow")) main = w;
    if (!main) return;
    if (!main->isActive()) {
        // It may be minimised or in the tray
        if (auto *tray = Registrar::instance<TrayController>()) tray->restore();
        main->requestActivate();
    }
    if (!asking(main)) emit opening();
}

double PaletteController::score(const QVariantMap &entry, const QString &query, QString *matched) {
    if (matched) matched->clear();
    const QStringList typed = words(query);
    if (typed.isEmpty()) return 1;
    const QString whole = typed.join(QLatin1Char(' '));

    const QString title = folded(entry.value(QStringLiteral("title")).toString());
    const QStringList aliases = entry.value(QStringLiteral("aliases")).toStringList();
    const QStringList keywords = entry.value(QStringLiteral("keywords")).toStringList();
    double sum = 0;
    QString by;   // the alias or keyword that was needed, the best one's
    double byScore = 0;
    int bySeveral = 0;
    for (const QString &word : typed) {
        double best = titleScore(word, title);
        QString from;
        for (const QString &alias : aliases)
            if (const double s = titleScore(word, folded(alias)) - 2; s > best) {
                best = s;
                from = alias;
            }
        for (const QString &keyword : keywords)
            if (const double s = keywordScore(word, folded(keyword)); s > best) {
                best = s;
                from = keyword;
            }
        if (best <= 0) return 0;
        sum += best;
        if (from.isEmpty()) continue;
        ++bySeveral;
        if (best > byScore) {
            by = from;
            byScore = best;
        }
    }
    // Rather one that has every word
    if (bySeveral > 1)
        for (const QString &text : aliases + keywords)
            if (std::all_of(typed.cbegin(), typed.cend(), [f = folded(text)](const QString &w) { return f.contains(w); })) {
                by = text;
                break;
            }
    double total = sum / double(typed.size());
    if (title == whole) total += 20;
    else if (title.startsWith(whole)) total += 10;
    else if (title.contains(whole)) total += 5;
    if (matched) *matched = by;
    return total;
}

QVariantList PaletteController::search(const QVariantList &entries, const QString &query) const {
    struct Found {
        QVariantMap entry;
        double score;
    };
    std::vector<Found> found;
    const bool any = !words(query).isEmpty();
    for (const QVariant &v : entries) {
        QVariantMap entry = v.toMap();
        QString matched;
        double s = score(entry, query, &matched);
        if (s <= 0) continue;
        if (any) {
            if (!entry.value(QStringLiteral("enabled")).toBool()) s -= 15;
            // Of equals the shorter title first
            s -= double(entry.value(QStringLiteral("title")).toString().size()) / 100;
        }
        entry.insert(QStringLiteral("matched"), matched);
        found.push_back({ entry, s });
    }
    if (any)
        std::stable_sort(found.begin(), found.end(), [](const Found &a, const Found &b) { return a.score > b.score; });
    QVariantList list;
    list.reserve(qsizetype(found.size()));
    for (const Found &f : found) list.append(f.entry);
    return list;
}
