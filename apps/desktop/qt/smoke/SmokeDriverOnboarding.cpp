// WP QD32: What's New and the Getting Started wizard show the core's words, with
// nothing left to resolve, on every step and for every state of the board step.
#include "SmokeDriver.h"

#include "OnboardingText.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQmlExpression>
#include <QQuickItem>
#include <QQuickWindow>
#include <QStringList>
#include <cstdio>

namespace {
QQuickItem *findItem(QQuickItem *from, const QString &name) {
    if (from->objectName() == name) return from;
    for (QQuickItem *c : from->childItems())
        if (QQuickItem *found = findItem(c, name)) return found;
    return nullptr;
}

// Every visible item's non-empty text
void shownTexts(QQuickItem *from, QStringList *out) {
    if (!from->isVisible()) return;
    const QString text = from->property("text").toString();
    if (!text.isEmpty()) out->append(text);
    for (QQuickItem *c : from->childItems()) shownTexts(c, out);
}

bool resolved(const QString &text) {
    return !text.contains(QLatin1String("[[")) && !text.contains(QLatin1Char('{'))
           && !text.contains(QLatin1String("undefined"));
}

bool allResolved(const QStringList &texts) {
    for (const QString &t : texts) {
        if (!resolved(t)) {
            printf("smoke:   unresolved text: %s\n", qPrintable(t));
            return false;
        }
    }
    return true;
}

QJsonObject stageWords(const QJsonObject &doc, const QString &id) {
    for (const QJsonValue &s : doc.value(QLatin1String("stages")).toArray())
        if (s.toObject().value(QLatin1String("id")).toString() == id) return s.toObject();
    return {};
}

// Every text of `list` is among the shown ones
bool allShown(const QJsonArray &list, const QString &joinedShown) {
    if (list.isEmpty()) return false;
    for (const QJsonValue &v : list) {
        const QString text = v.toObject().value(QLatin1String("text")).toString();
        if (text.isEmpty() || !joinedShown.contains(text)) {
            printf("smoke:   not shown: %s\n", qPrintable(text));
            return false;
        }
    }
    return true;
}
}

void SmokeDriver::testOnboarding() {
    section("onboarding: the core's words (QD32)");
    auto *text = qobject_cast<OnboardingText *>(
        m_.engine->rootContext()->contextProperty(QStringLiteral("onboardingText")).value<QObject *>());
    check("the controller is registered", text != nullptr);
    if (!text) return;

    // What's New, as the window gets it
    const QVariantList notes = text->releaseNotes();
    check("release notes", !notes.isEmpty());
    const QString notesJson = QString::fromUtf8(QJsonDocument::fromVariant(notes).toJson());
    check("release notes have nothing left to resolve", !notesJson.contains(QLatin1String("[[")));
    check("the 1.1.6-beta4 notes are there", notesJson.contains(QLatin1String("1.1.6-beta4")));
#ifndef Q_OS_MACOS
    check("the menu path is this app's", notesJson.contains(QLatin1String("Effects > Tube Modeller"))
                                             && !notesJson.contains(QLatin1String("Tools > Tube Modeller")));
#endif

    // The wizard, as main.qml shows it
    root()->setProperty("setupRequested", true);
    spin(300);
    QQuickItem *rootItem = qobject_cast<QQuickItem *>(root());
    if (auto *window = qobject_cast<QQuickWindow *>(root())) rootItem = window->contentItem();
    QQuickItem *wiz = rootItem ? findItem(rootItem, QStringLiteral("gettingStarted")) : nullptr;
    check("the wizard opens", wiz != nullptr);
    if (!wiz) {
        root()->setProperty("setupRequested", false);
        return;
    }
    const QJsonObject words = QJsonObject::fromVariantMap(text->gettingStarted());
    check("the wizard has its stages", wiz->property("stages").toStringList()
                                           == QStringList({ "Welcome", "Board", "Done" }));

    QStringList shown;
    shownTexts(wiz, &shown);
    QString joined = shown.join(QLatin1Char('\n'));
    const QJsonObject welcome = stageWords(words, QStringLiteral("welcome"));
    check("Welcome is resolved", allResolved(shown));
    check("Welcome shows the core's paragraphs", allShown(welcome.value(QLatin1String("paragraphs")).toArray(), joined));
    check("Welcome shows the core's rows", allShown(welcome.value(QLatin1String("rows")).toArray(), joined));
    check("the step counter is filled in", joined.contains(QLatin1String("Step 1 of 3")));
    check("the skip button has its word", shown.contains(QLatin1String("Skip Setup")));

    wiz->setProperty("stage", 1);
    spin(100);
    shown.clear();
    shownTexts(wiz, &shown);
    check("Board is resolved", allResolved(shown));

    // Every state the board step has, whatever the demo device is doing now
    QQmlContext *context = QQmlEngine::contextForObject(wiz);
    const auto eval = [&](const QString &js) {
        QQmlExpression e(context, wiz, js);
        return e.evaluate().toString();
    };
    bool cardsOk = true;
    for (const char *phase : { "idle", "waitingForBoard", "waitingForVolume", "ready", "waitingForDevice", "verified", "failed",
                               /* -- WP QD11 -- */ "checking", "waitingForDrive", "unconfirmed", "inBootloader" }) {
        for (const char *confirmed : { "false", "true" }) {
            for (int match = 0; match <= 3; match++) {
                for (const char *mundane : { "false", "true" }) {
                    const QString js = QStringLiteral("JSON.stringify(cardFor('%1', %2, %3, %4))")
                                           .arg(QLatin1String(phase), QLatin1String(confirmed)).arg(match).arg(QLatin1String(mundane));
                    const QJsonObject card = QJsonDocument::fromJson(eval(js).toUtf8()).object();
                    const QString title = card.value(QLatin1String("title")).toString();
                    // "failed" fills in the reason, empty until a run fails
                    const QString body = card.value(QLatin1String("text")).toString();
                    if (title.isEmpty() || (body.isEmpty() && QLatin1String(phase) != QLatin1String("failed") && QLatin1String(phase) != QLatin1String("unconfirmed") && QLatin1String(phase) != QLatin1String("inBootloader"))
                        || !resolved(title) || !resolved(body)) {
                        printf("smoke:   bad card: %s -> %s\n", qPrintable(js), qPrintable(eval(js)));
                        cardsOk = false;
                    }
                }
            }
        }
    }
    check("every card has its words", cardsOk);
    bool headersOk = true;
    for (const char *phase : { "idle", "writing", "waitingForDevice", "verified", "failed", /* -- WP QD11 -- */ "question", "unconfirmed", "inBootloader" }) {
        for (int match = 0; match <= 3; match++) {
            const QString js = QStringLiteral("JSON.stringify(headerFor('%1', %2))").arg(QLatin1String(phase)).arg(match);
            const QJsonObject header = QJsonDocument::fromJson(eval(js).toUtf8()).object();
            if (header.value(QLatin1String("title")).toString().isEmpty()
                || header.value(QLatin1String("blurb")).toString().isEmpty()) {
                printf("smoke:   bad header: %s\n", qPrintable(js));
                headersOk = false;
            }
        }
    }
    check("every board header has its words", headersOk);
    check("the writing card has its words",
          !eval(QStringLiteral("byId(boardWords.cards, 'card.writing').title")).isEmpty());

    wiz->setProperty("stage", 2);
    spin(100);
    shown.clear();
    shownTexts(wiz, &shown);
    joined = shown.join(QLatin1Char('\n'));
    const QJsonObject done = stageWords(words, QStringLiteral("done"));
    check("Done is resolved", allResolved(shown));
    check("Done shows the core's paragraphs", allShown(done.value(QLatin1String("paragraphs")).toArray(), joined));
    check("Done shows the core's rows", allShown(done.value(QLatin1String("rows")).toArray(), joined));
    check("the finish button has its word", shown.contains(QLatin1String("Start Using DSPi uConsole")));
#ifndef Q_OS_MACOS
    check("no host device on Done", !joined.contains(QLatin1String("computer")));
    check("Done names this app's settings page", joined.contains(QLatin1String("under Outputs")));
#endif

    wiz->setProperty("stage", 0);
    spin(50);
    root()->setProperty("setupRequested", false);
    spin(100);
}
