// -- WP QP15 -- AutoEQ's section: the built-in database, the browser's search
// and empty states, favourites in the menu, applying a profile to inputs 1
// and 2, the Update Database dialog, importing a database and resetting to the
// built-in one, a rebuild against a scripted GitHub, and the real fetch
// against a server on the loopback interface. Nothing reaches a real network;
// every change is undone before the section ends.
#include "Qp18Notices.h"   // -- WP QP18 --
#include "SmokeDriver.h"

#include "AutoEqController.h"
#include "DeviceModel.h"
#include "FilesController.h"
#include "Registrar.h"
#include "Shell.h"

#include "CoreApi.h"

#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QHostAddress>
#include <QQuickItem>
#include <QQuickWindow>
#include <QSettings>
#include <QTcpServer>
#include <QTcpSocket>
#include <QThread>
#include <atomic>
#include <memory>

namespace {

const QByteArray kApi = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/";
const QByteArray kRaw = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/";

bool opened(QObject *popup) { return popup && popup->property("opened").toBool(); }

QString takeNoticeText(SmokeDriver &d, QObject *notice, bool *error = nullptr, int ms = 8000) {
    // -- WP QP18 -- or a success's notice in the banner stack
    if (!notice || !d.waitFor(ms, [&]() { return opened(notice) || qp18NoticeNew(); })) return {};
    if (!opened(notice)) {
        if (error) *error = false;
        return qp18TakeNotice();
    }
    const QString text = notice->property("text").toString();
    if (error) *error = notice->property("error").toBool();
    QMetaObject::invokeMethod(notice, "close");
    d.waitFor(2000, [&]() { return !opened(notice); });
    return text;
}

QQuickWindow *topWindow(const QString &name) {
    for (QWindow *w : QGuiApplication::topLevelWindows())
        if (w->objectName() == name) return qobject_cast<QQuickWindow *>(w);
    return nullptr;
}

AutoEqController::Answer reply(int status, const QByteArray &body) {
    AutoEqController::Answer a;
    a.answered = true;
    a.status = status;
    a.body = body;
    return a;
}

QByteArray profileUrl(const QByteArray &headphone) {
    QByteArray h = headphone;
    h.replace(' ', "%20");
    return kRaw + "oratory1990/over-ear/" + h + "/" + h + "%20ParametricEQ.txt";
}

} // namespace

static void autoEqSection(SmokeDriver &d) {
    qp18NoticeMark();   // -- WP QP18 --
    DeviceModel *device = d.app().device;
    Shell *shell = d.app().shell;
    AutoEqController *aeq = Registrar::instance<AutoEqController>();
    FilesController *files = Registrar::instance<FilesController>();
    d.check(QStringLiteral("qp15: the AutoEQ controller is made"), aeq && files);
    if (!aeq || !files) return;
    QQuickItem *host = files->host();
    QObject *notice = host ? host->findChild<QObject *>(QStringLiteral("FilesHost.notice")) : nullptr;
    const QString dir = d.outDir() + QStringLiteral("/qp15");
    QDir().mkpath(dir);

    // ---- The built-in database ----
    d.check(QStringLiteral("qp15: the built-in database loads"),
            d.waitFor(15000, [&]() { return !aeq->loading(); }) && aeq->loadProblem().isEmpty());
    const int builtIn = aeq->profileCount();
    d.check(QStringLiteral("qp15: it holds the shipped profiles"), builtIn == 3828);
    d.check(QStringLiteral("qp15: it has a date and no user file"), !aeq->generatedOn().isEmpty() && !aeq->updatedDatabase());

    // ---- The browser ----
    shell->trigger(QStringLiteral("autoeq.browse"));
    QQuickWindow *browser = nullptr;
    d.check(QStringLiteral("qp15: Browse Profiles opens the browser"),
            d.waitFor(3000, [&]() { return (browser = topWindow(QStringLiteral("AutoEqWindow"))) && browser->isVisible(); }));
    if (!browser) return;
    d.check(QStringLiteral("qp15: the browser's title and size are the Mac's"),
            browser->title() == QLatin1String("AutoEQ - Browse Profiles") && browser->minimumWidth() == 450);
    AutoEqProfileModel *list = aeq->profiles();
    d.check(QStringLiteral("qp15: every profile is listed"), list->rowCount() == builtIn);
    list->setSearchText(QStringLiteral("HD 600"));
    bool allMatch = list->rowCount() > 0;
    for (int r = 0; r < list->rowCount(); ++r)
        allMatch = allMatch && list->data(list->index(r), AutoEqProfileModel::DisplayName).toString().contains(QLatin1String("HD 600"), Qt::CaseInsensitive);
    d.check(QStringLiteral("qp15: the search narrows the list"), allMatch && list->rowCount() < builtIn);
    const QString id = list->idAt(0);
    browser->setProperty("selectedId", id);
    QObject *apply = browser->findChild<QObject *>(QStringLiteral("AutoEqWindow.apply"));
    d.check(QStringLiteral("qp15: a selection enables Apply"), apply && apply->property("enabled").toBool());
    QObject *selected = browser->findChild<QObject *>(QStringLiteral("AutoEqWindow.selectedName"));
    d.check(QStringLiteral("qp15: the bottom bar names it"), selected && selected->property("text").toString() == aeq->profile(id).value(QStringLiteral("name")).toString());
    d.spin(300);
    d.check(QStringLiteral("qp15: screenshot of the browser"), d.grab(browser, QStringLiteral("window-autoeq.png")));

    list->setSearchText(QStringLiteral("zzzz no such headphone"));
    QObject *empty = browser->findChild<QObject *>(QStringLiteral("AutoEqWindow.empty"));
    d.check(QStringLiteral("qp15: no match says so"),
            list->rowCount() == 0 && empty && empty->property("text").toString() == QLatin1String("No headphones found matching \"zzzz no such headphone\""));
    d.check(QStringLiteral("qp15: a profile the search hides is not applied"), browser->property("selectedId").toString().isEmpty());
    list->setSearchText(QString());

    // ---- Apply: the profile on inputs 1 and 2 ----
    const DspState before = *device->state();
    browser->setProperty("selectedId", id);
    QMetaObject::invokeMethod(browser, "applySelected");
    d.check(QStringLiteral("qp15: Apply closes the browser"), d.waitFor(2000, [&]() { return !browser->isVisible(); }));
    d.waitIdle();
    QFile builtInFile(AutoEqController::builtInDatabasePath());
    d.check(QStringLiteral("qp15: the built-in database is in the app"), builtInFile.exists());
    const DspState *s = device->state();
    bool bandsSent = s->filters[0][0].filter_type != FILTER_FLAT || s->filters[0][1].filter_type != FILTER_FLAT;
    bool pairSame = true;
    for (int b = 0; b < BANDS_PER_CHANNEL; ++b)
        pairSame = pairSame && s->filters[0][b].filter_type == s->filters[1][b].filter_type && s->filters[0][b].freq == s->filters[1][b].freq &&
                   s->filters[0][b].gain == s->filters[1][b].gain;
    d.check(QStringLiteral("qp15: the profile's bands are on inputs 1 and 2"), bandsSent && pairSame);
    d.check(QStringLiteral("qp15: and its preamp, on both"), s->input_preamp_db[0] == s->input_preamp_db[1] && s->input_preamp_db[0] < 0);
    // Back as they were
    device->write(QStringLiteral("qp15 autoeq restore"), [before](FfiCore *c) {
        bool ok = true;
        for (int ch = 0; ch < 2; ++ch) {
            ok = dspi_set_input_preamp(c, uint8_t(ch), before.input_preamp_db[ch]) && ok;
            for (int b = 0; b < BANDS_PER_CHANNEL; ++b) ok = dspi_set_filter(c, uint8_t(ch), uint8_t(b), before.filters[ch][b]) && ok;
        }
        return ok;
    });
    d.waitIdle();

    // ---- Favourites and their menu ----
    d.check(QStringLiteral("qp15: no favourites at first"),
            aeq->favouriteList().isEmpty() && d.object(QStringLiteral("Menu.autoeq.noFavorites")) != nullptr);
    aeq->flipFavourite(id);
    d.check(QStringLiteral("qp15: the heart adds a favourite"), aeq->isFavourite(id) && aeq->favouriteList().size() == 1);
    d.check(QStringLiteral("qp15: it is stored"), QSettings().value(QStringLiteral("autoeq/favorites")).toStringList() == QStringList{ id });
    d.spin(100);
    QObject *row = d.object(QStringLiteral("Menu.autoeq.favorite.0"));
    d.check(QStringLiteral("qp15: the menu lists it"),
            row && row->property("text").toString() == aeq->favouriteList().first().toMap().value(QStringLiteral("title")).toString());
    d.check(QStringLiteral("qp15: No favouriteList yet is gone"), d.object(QStringLiteral("Menu.autoeq.noFavorites")) == nullptr);
    QObject *clear = d.object(QStringLiteral("Menu.autoeq.clearFavorites"));
    d.check(QStringLiteral("qp15: Clear Favorites is enabled"), clear && clear->property("enabled").toBool());
    shell->trigger(QStringLiteral("autoeq.clearFavorites"));
    d.spin(100);
    d.check(QStringLiteral("qp15: Clear Favorites empties them"),
            aeq->favouriteList().isEmpty() && !QSettings().contains(QStringLiteral("autoeq/favorites")) &&
                d.object(QStringLiteral("Menu.autoeq.noFavorites")) != nullptr && clear && !clear->property("enabled").toBool());

    // ---- Update Database... ----
    QObject *update = host ? host->findChild<QObject *>(QStringLiteral("AutoEqUpdateDialog")) : nullptr;
    shell->trigger(QStringLiteral("autoeq.updateDatabase"));
    d.check(QStringLiteral("qp15: Update Database shows its choices"), d.waitFor(2000, [&]() { return opened(update); }));
    d.check(QStringLiteral("qp15: with the database in use"),
            update && update->property("text").toString().contains(QStringLiteral("Entries: %1").arg(builtIn)));
    QObject *reset = host->findChild<QObject *>(QStringLiteral("AutoEqUpdateDialog.reset"));
    d.check(QStringLiteral("qp15: no Reset to Built-in on the built-in database"), reset && !reset->property("visible").toBool());
    d.spin(200);
    d.check(QStringLiteral("qp15: screenshot of Update Database"), d.grab(d.mainWindow(), QStringLiteral("window-autoeq-update.png")));
    QMetaObject::invokeMethod(update, "close");
    d.waitFor(2000, [&]() { return !opened(update); });

    // ---- Import a database file, then Reset to Built-in ----
    const QString small = dir + QStringLiteral("/autoeq_small.json");
    {
        QFile f(small);
        f.open(QIODevice::WriteOnly | QIODevice::Truncate);
        f.write(R"({"version":1,"generatedAt":"2026-02-03T04:05:06Z","entryCount":2,"entries":[
            {"id":"oratory1990/Acme One","manufacturer":"Acme","model":"One","source":"oratory1990","formFactor":"over-ear",
             "preamp":-4.0,"filters":[{"type":"peaking","freq":100.0,"q":1.0,"gain":3.0}]},
            {"id":"crinacle/Acme Bud","manufacturer":"Acme","model":"Bud","source":"crinacle","formFactor":"in-ear",
             "preamp":-2.0,"filters":[{"type":"highShelf","freq":8000.0,"q":0.7,"gain":-2.0}]}]})");
    }
    aeq->importDatabaseFrom(small);
    bool error = true;
    QString text = takeNoticeText(d, notice, &error);
    d.check(QStringLiteral("qp15: importing a database says how many it holds"),
            !error && text == QLatin1String("Database updated successfully.\nEntries: 2"));
    d.check(QStringLiteral("qp15: the imported database is in use"), aeq->profileCount() == 2 && aeq->updatedDatabase());
    shell->trigger(QStringLiteral("autoeq.updateDatabase"));
    d.waitFor(2000, [&]() { return opened(update); });
    d.check(QStringLiteral("qp15: Reset to Built-in is offered now"), reset->property("visible").toBool());
    QMetaObject::invokeMethod(reset, "clicked");
    text = takeNoticeText(d, notice, &error);
    d.check(QStringLiteral("qp15: Reset to Built-in says so"),
            !error && text == QStringLiteral("Reset to built-in database.\nEntries: ") + QString::number(builtIn));
    d.check(QStringLiteral("qp15: the built-in database is in use again"), aeq->profileCount() == builtIn && !aeq->updatedDatabase());
    const QString junk = dir + QStringLiteral("/junk.json");
    {
        QFile f(junk);
        f.open(QIODevice::WriteOnly | QIODevice::Truncate);
        f.write("not a database");
    }
    aeq->importDatabaseFrom(junk);
    text = takeNoticeText(d, notice, &error);
    d.check(QStringLiteral("qp15: a file that is no database is refused"),
            error && text.startsWith(QLatin1String("Failed to import database: ")) && aeq->profileCount() == builtIn && !aeq->updatedDatabase());

    // ---- Rebuild from GitHub, against a scripted GitHub ----
    auto asked = std::make_shared<std::atomic<int>>(0);
    auto agentSeen = std::make_shared<std::atomic<bool>>(true);
    AutoEqController::setFetchOverride([asked, agentSeen](const QByteArray &url, const QByteArray &, const QByteArray &agent) {
        asked->fetch_add(1);
        if (agent.isEmpty()) agentSeen->store(false);
        QThread::msleep(20);
        if (url == kApi + "oratory1990") return reply(200, R"([{"name":"over-ear","type":"dir"}])");
        if (url == kApi + "oratory1990/over-ear")
            return reply(200, R"([{"name":"Sennheiser HD 600","type":"dir"},{"name":"Sony WH-1000XM4","type":"dir"}])");
        if (url == profileUrl("Sennheiser HD 600"))
            return reply(200, "Preamp: -6.5 dB\nFilter 1: ON PK Fc 123 Hz Gain 3 dB Q 0.7\nFilter 2: ON LSC Fc 105 Hz Gain 2 dB Q 0.71\n");
        if (url == profileUrl("Sony WH-1000XM4"))
            return reply(200, "Preamp: -3.0 dB\nFilter 1: ON HSC Fc 9000 Hz Gain -2 dB Q 0.7\n");
        return reply(404, QByteArray());
    });
    QObject *confirm = host->findChild<QObject *>(QStringLiteral("FilesHost.rebuildQuestion"));
    shell->trigger(QStringLiteral("autoeq.updateDatabase"));
    d.waitFor(2000, [&]() { return opened(update); });
    QMetaObject::invokeMethod(host->findChild<QObject *>(QStringLiteral("AutoEqUpdateDialog.rebuild")), "clicked");
    d.check(QStringLiteral("qp15: Rebuild from GitHub asks to confirm"),
            d.waitFor(2000, [&]() { return opened(confirm); }) &&
                confirm->property("text").toString().startsWith(QLatin1String("You are about to rebuild the AutoEQ database")));
    QMetaObject::invokeMethod(confirm, "finish", Q_ARG(QVariant, QStringLiteral("accept")));
    QQuickWindow *sheet = nullptr;
    d.check(QStringLiteral("qp15: the Updating Database window shows"),
            d.waitFor(3000, [&]() { return (sheet = topWindow(QStringLiteral("AutoEqRebuild"))) && sheet->isVisible(); }));
    d.check(QStringLiteral("qp15: its title is the Mac's"), sheet && sheet->title() == QLatin1String("Updating Database"));
    d.check(QStringLiteral("qp15: the rebuild finishes"), d.waitFor(20000, [&]() { return aeq->rebuildFinished(); }));
    d.spin(300);
    if (sheet) d.check(QStringLiteral("qp15: screenshot of the finished rebuild"), d.grab(sheet, QStringLiteral("window-autoeqRebuild.png")));
    text = takeNoticeText(d, notice, &error, 8000);
    d.check(QStringLiteral("qp15: the rebuild says how many profiles it found"),
            !error && text == QLatin1String("Database rebuilt successfully!\nEntries: 2"));
    d.check(QStringLiteral("qp15: the window goes when it is done"), !aeq->rebuildOnScreen() && (!sheet || !sheet->isVisible()));
    d.check(QStringLiteral("qp15: the rebuilt database is in use"), aeq->profileCount() == 2 && aeq->updatedDatabase());
    d.check(QStringLiteral("qp15: every request carried a user agent"), asked->load() >= 4 && agentSeen->load());
    list->setSearchText(QStringLiteral("HD 600"));
    d.check(QStringLiteral("qp15: the rebuilt profile is found"), list->rowCount() == 1);
    list->setSearchText(QString());

    // A cancelled rebuild writes nothing and says nothing
    AutoEqController::setFetchOverride([](const QByteArray &, const QByteArray &, const QByteArray &) {
        QThread::msleep(300);
        return reply(404, QByteArray());
    });
    aeq->rebuildFromGitHub();
    d.waitFor(1000, [&]() { return aeq->rebuildOnScreen(); });
    aeq->stopRebuild();
    d.check(QStringLiteral("qp15: Cancel stops the rebuild"), d.waitFor(10000, [&]() { return !aeq->rebuildOnScreen(); }));
    d.spin(200);
    d.check(QStringLiteral("qp15: and leaves the database alone"), aeq->profileCount() == 2 && !opened(notice));
    AutoEqController::setFetchOverride({});

    aeq->resetToBuiltIn();
    takeNoticeText(d, notice);
    d.check(QStringLiteral("qp15: back on the built-in database"), aeq->profileCount() == builtIn && !aeq->updatedDatabase());

    // ---- The real fetch, against a server on the loopback interface ----
    QTcpServer server;
    QByteArray request;
    server.listen(QHostAddress::LocalHost);
    QObject::connect(&server, &QTcpServer::newConnection, &server, [&]() {
        QTcpSocket *socket = server.nextPendingConnection();
        QObject::connect(socket, &QTcpSocket::readyRead, socket, [&, socket]() {
            request += socket->readAll();
            if (!request.contains("\r\n\r\n")) return;
            socket->write("HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nRetry-After: 42\r\n"
                          "X-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 1767225600\r\nContent-Length: 13\r\n"
                          "Connection: close\r\n\r\n{\"rate\":true}");
            socket->disconnectFromHost();
        });
    });
    const QByteArray url = "http://127.0.0.1:" + QByteArray::number(server.serverPort()) + "/repos/x";
    AutoEqController::Answer got;
    std::atomic<bool> done{ false };
    QThread *fetcher = QThread::create([&]() {
        got = AutoEqController::fetch(url, "application/vnd.github.v3+json", "uConsole-smoke/1");
        done = true;
    });
    fetcher->start();
    d.waitFor(10000, [&]() { return done.load(); });
    fetcher->wait(2000);
    delete fetcher;
    d.check(QStringLiteral("qp15: the fetch returns an error status as an answer"),
            got.answered && got.status == 403 && got.body == "{\"rate\":true}");
    d.check(QStringLiteral("qp15: with the rate-limit headers"), got.retryAfter == 42 && got.remaining == 0 && got.reset == 1767225600);
    d.check(QStringLiteral("qp15: the fetch sends the user agent and Accept"),
            request.contains("User-Agent: uConsole-smoke/1") && request.contains("Accept: application/vnd.github.v3+json"));
    server.close();
    done = false;
    fetcher = QThread::create([&]() {
        got = AutoEqController::fetch(url, QByteArray(), "uConsole-smoke/1");
        done = true;
    });
    fetcher->start();
    d.waitFor(10000, [&]() { return done.load(); });
    fetcher->wait(2000);
    delete fetcher;
    d.check(QStringLiteral("qp15: no server is no answer, with words for it"), !got.answered && !got.error.isEmpty());
}

static const SmokeDriver::Section autoEq("qp15-autoeq", 151, &autoEqSection);
