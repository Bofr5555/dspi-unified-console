// WP QD27: updating the AutoEQ database through the core, with Qt's fetch.
// Scripted answers stand in for GitHub; the real fetch is tried against a server
// on the loopback interface. Nothing here reaches a real network.
#include "SmokeDriver.h"

#include "AutoEqLibrary.h"
#include "DSPiBridge.h"

#include <QAbstractListModel>
#include <QCoreApplication>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QHash>
#include <QMutex>
#include <QQuickItem>
#include <QTcpServer>
#include <QTcpSocket>
#include <QThread>
#include <atomic>
#include <cstdio>
#include <memory>
#include <thread>

namespace {
const QByteArray kApi = "https://api.github.com/repos/jaakkopasanen/AutoEq/contents/results/";
const QByteArray kRaw = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/";
const QByteArray kProfile =
    "Preamp: -6.5 dB\nFilter 1: ON PK Fc 123 Hz Gain 3 dB Q 0.7\nFilter 2: ON LSC Fc 105 Hz Gain 2 dB Q 0.71\n";
const QByteArray kDatabase = R"({"version":1,"generatedAt":"2026-01-02T03:04:05.000Z","entryCount":1,
    "entries":[{"id":"o/Sony A","manufacturer":"Sony","model":"A","filters":[]}]})";

AutoEqFetchAnswer answer(int status, const QByteArray &body = QByteArray()) {
    AutoEqFetchAnswer a;
    a.answered = true;
    a.status = status;
    a.body = body;
    return a;
}

QByteArray profileUrl(const QByteArray &target, const QByteArray &headphone) {
    QByteArray h = headphone;
    h.replace(' ', "%20");
    return kRaw + "oratory1990/" + target + "/" + h + "/" + h + "%20ParametricEQ.txt";
}

QByteArray listing(const QList<QByteArray> &names) {
    QByteArrayList items;
    for (const QByteArray &n : names) items << R"({"name":")" + n + R"(","type":"dir"})";
    return "[" + items.join(",") + "]";
}

// A scripted GitHub: a URL nobody scripted is a 404, as it would be there
struct Script {
    struct Asked { QByteArray url, accept, agent; };
    QHash<QByteArray, AutoEqFetchAnswer> answers;
    QMutex mutex;
    QVector<Asked> asked;
    AutoEqFetchAnswer get(const QByteArray &url, const QByteArray &accept, const QByteArray &agent) {
        QMutexLocker lock(&mutex);
        asked.append({ url, accept, agent });
        return answers.value(url, answer(404));
    }
};

AutoEqLibrary::Fetcher fetcherOf(const std::shared_ptr<Script> &script) {
    return [script](const QByteArray &url, const QByteArray &accept, const QByteArray &agent, const std::atomic_bool &) {
        return script->get(url, accept, agent);
    };
}

QByteArray fileBytes(const QString &path) {
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray("<no file>");
}

// Answers from a thread of its own, so it goes on while the GUI thread is held.
// The handler gives the raw reply; empty leaves the request hanging.
class LoopServer : public QThread {
public:
    struct Request { QByteArray path, accept, agent; };
    using Handler = std::function<QByteArray(const QByteArray &path)>;
    explicit LoopServer(const Handler &handler) : m_handler(handler) {
        start();
        while (!m_port) QThread::msleep(5);
    }
    ~LoopServer() override { quit(); wait(); }
    QByteArray base() const { return "http://127.0.0.1:" + QByteArray::number(int(m_port)); }
    int served() const { return m_served; }
    QVector<Request> requests() { QMutexLocker lock(&m_mutex); return m_requests; }

protected:
    void run() override {
        QTcpServer server;
        server.listen(QHostAddress::LocalHost);
        QObject::connect(&server, &QTcpServer::newConnection, &server, [this, &server]() {
            while (QTcpSocket *c = server.nextPendingConnection()) {
                c->setParent(&server);
                QObject::connect(c, &QTcpSocket::readyRead, c, [this, c]() { read(c); });
                QObject::connect(c, &QTcpSocket::disconnected, c, &QObject::deleteLater);
            }
        });
        m_port = server.serverPort();
        exec();
    }

private:
    void read(QTcpSocket *c) {
        QByteArray &text = m_buffers[c];
        text += c->readAll();
        if (!text.contains("\r\n\r\n")) return;
        const QList<QByteArray> lines = text.split('\n');
        Request r;
        r.path = lines.value(0).split(' ').value(1);
        for (const QByteArray &line : lines) {
            const QByteArray lower = line.toLower();
            if (lower.startsWith("user-agent:")) r.agent = line.mid(11).trimmed();
            if (lower.startsWith("accept:")) r.accept = line.mid(7).trimmed();
        }
        m_buffers.remove(c);
        {
            QMutexLocker lock(&m_mutex);
            m_requests.append(r);
        }
        const QByteArray reply = m_handler(r.path);
        m_served++;
        if (reply.isEmpty()) return;
        c->write(reply);
        c->disconnectFromHost();
    }
    Handler m_handler;
    std::atomic<quint16> m_port{ 0 };
    std::atomic_int m_served{ 0 };
    QHash<QTcpSocket *, QByteArray> m_buffers;
    QMutex m_mutex;
    QVector<Request> m_requests;
};

QByteArray http(const QByteArray &statusLine, const QByteArray &headers, const QByteArray &body) {
    return "HTTP/1.1 " + statusLine + "\r\n" + headers + "Content-Length: " + QByteArray::number(body.size())
        + "\r\nConnection: close\r\n\r\n" + body;
}
}

void SmokeDriver::testAutoEqUpdate() {
    section("AutoEQ update (QD27)");
    auto *a = m_.autoeq;
    auto *b = m_.bridge;
    const QString db = a->userPathForTest();
    a->load();
    const int builtIn = a->entryCount();
    check("qd27: starts on the built-in database", builtIn > 0 && !a->hasUserDatabase());

    // The dialog a result is shown in; closed again so that it blocks nothing
    const auto resultShown = [this](const QString &title, const QString &part) {
        QObject *shown = nullptr;
        waitFor(2000, [&]() {
            for (QObject *o : root()->findChildren<QObject *>())
                if (o->inherits("QQuickPopup") && o->property("visible").toBool() && o->property("title").toString() == title)
                    shown = o;
            return shown != nullptr;
        });
        if (!shown) {
            printf("smoke: qd27: no dialog titled %s\n", qPrintable(title));
            return false;
        }
        const bool match = shown->property("message").toString().contains(part);
        if (!match) printf("smoke: qd27 dialog says: %s\n", qPrintable(shown->property("message").toString()));
        QMetaObject::invokeMethod(shown, "close");
        spin(300);
        return match;
    };

    int finished = 0;
    bool finishedOk = false;
    QString finishedMessage;
    const QMetaObject::Connection rebuilt = connect(a, &AutoEqLibrary::rebuildFinished, this, [&](bool ok, const QString &m) {
        finished++;
        finishedOk = ok;
        finishedMessage = m;
    });
    int updates = 0;
    bool updateOk = false;
    QString updateMessage;
    const QMetaObject::Connection updated = connect(a, &AutoEqLibrary::updateFinished, this, [&](bool ok, const QString &, const QString &m) {
        updates++;
        updateOk = ok;
        updateMessage = m;
    });

    const QString warning = a->rebuildWarning();
    check("qd27: the warning names the hosts and the file", warning.contains("api.github.com") && warning.contains(db));

    // Partly failed: the database is written, and the person told what is missing
    auto script = std::make_shared<Script>();
    script->answers[kApi + "oratory1990"] = answer(200, listing({ "t", "broken" }));
    script->answers[kApi + "oratory1990/broken"] = answer(500);
    script->answers[kApi + "oratory1990/t"] = answer(200, listing({ "Sony A", "Sony B", "Sony C", "Sony D", "Sony E" }));
    for (const char *h : { "Sony A", "Sony B", "Sony C" }) script->answers[profileUrl("t", h)] = answer(200, kProfile);
    AutoEqFetchAnswer noAnswer;
    noAnswer.failure = "timed out";
    script->answers[profileUrl("t", "Sony D")] = noAnswer;  // Sony E: a 404, which is normal upstream
    a->setFetcherForTest(fetcherOf(script));
    a->startRebuild();
    check("qd27: rebuilding", a->rebuilding());
    check("qd27: partial rebuild ends", waitFor(15000, [&]() { return finished == 1; }));
    check("qd27: partial rebuild succeeds", finishedOk);
    printf("smoke: qd27 partial: %s\n", qPrintable(finishedMessage));
    check("qd27: says the entries", finishedMessage.contains("Entries: 3"));
    check("qd27: shows failedProfiles", finishedMessage.contains("1 profile could not be downloaded"));
    check("qd27: shows failedListings", finishedMessage.contains("1 folder could not be listed"));
    check("qd27: the result dialog shows them", resultShown("AutoEQ Database", "1 folder could not be listed"));
    check("qd27: not rebuilding after", !a->rebuilding() && a->rebuildProgress() == 1.0);
    check("qd27: the rebuilt database is loaded", a->entryCount() == 3 && a->hasUserDatabase() && a->loadError().isEmpty());
    {
        QMutexLocker lock(&script->mutex);
        bool agents = !script->asked.isEmpty(), accepts = true;
        for (const Script::Asked &r : script->asked) {
            agents = agents && r.agent == "DSPi-Console";
            accepts = accepts && r.accept == (r.url.startsWith(kApi) ? QByteArray("application/vnd.github.v3+json") : QByteArray());
        }
        check("qd27: the fetch is given the User-Agent", agents);
        check("qd27: and Accept for listings only", accepts);
    }

    // What the core wrote is what browsing and applying read
    a->setQuery(QStringLiteral("Sony A"));
    auto *results = qobject_cast<QAbstractListModel *>(a->results());
    check("qd27: search finds a rebuilt profile", results && results->rowCount() == 1);
    const QString id = results ? results->index(0).data(AutoEqModel::IdRole).toString() : QString();
    a->setQuery(QString());
    check("qd27: entry", a->entry(id).value("name").toString() == "Sony A");
    a->toggleFavorite(id);
    check("qd27: favourite", a->favorites().size() == 1);
    a->clearFavorites();
    check("qd27: favourites cleared", a->favorites().isEmpty());
    struct Band { int type; float freq, gain, q; };
    QVector<Band> before;
    for (int ch = 0; ch < 2; ch++)
        for (int band = 0; band < 10; band++)
            before.append({ b->filterType(ch, band), b->filterFreq(ch, band), b->filterGain(ch, band), b->filterQ(ch, band) });
    const float preampBefore[2] = { b->inputPreampDB(0), b->inputPreampDB(1) };
    check("qd27: apply", a->apply(id));
    check("qd27: applied", waitFor(3000, [&]() {
        return qAbs(b->filterFreq(1, 0) - 123.0f) < 0.5f && b->filterType(1, 0) == 1 && b->filterType(1, 1) == 2
            && b->filterType(1, 2) == 0 && qAbs(b->inputPreampDB(1) + 6.5f) < 0.1f;
    }));
    for (int i = 0; i < before.size(); i++)
        b->setFilter(i / 10, i % 10, before[i].type, before[i].freq, before[i].gain, before[i].q);
    for (int input = 0; input < 2; input++) b->setInputPreamp(input, preampBefore[input]);
    spin(300);

    // A rate limit: the file is left, and the person told when to come back
    QByteArray kept = fileBytes(db);
    auto limited = std::make_shared<Script>();
    AutoEqFetchAnswer limit = answer(403);
    limit.rateLimitRemaining = 0;
    limit.retryAfterS = 1200;
    limited->answers[kApi + "oratory1990"] = limit;
    a->setFetcherForTest(fetcherOf(limited));
    finished = 0;
    a->startRebuild();
    check("qd27: rate limit ends", waitFor(15000, [&]() { return finished == 1; }));
    printf("smoke: qd27 limit: %s\n", qPrintable(finishedMessage));
    check("qd27: rate limit fails", !finishedOk && finishedMessage.startsWith("Rebuild failed:"));
    check("qd27: says when", finishedMessage.contains("about 20 minutes"));
    check("qd27: rate limit left the file", fileBytes(db) == kept);
    check("qd27: the failure dialog shows it", resultShown("Rebuild Failed", "about 20 minutes"));

    // Too much failed: nothing is written
    auto failing = std::make_shared<Script>();
    failing->answers[kApi + "oratory1990"] = answer(500);
    a->setFetcherForTest(fetcherOf(failing));
    finished = 0;
    a->startRebuild();
    check("qd27: no profiles ends", waitFor(15000, [&]() { return finished == 1; }));
    check("qd27: no profiles fails", !finishedOk);
    check("qd27: no profiles left the file", fileBytes(db) == kept);
    check("qd27: its dialog", resultShown("Rebuild Failed", ""));

    // Cancel: at once for the caller, nothing written, nothing reported, and a
    // new rebuild can start straight away
    std::atomic_int entered{ 0 }, left{ 0 };
    auto restart = std::make_shared<Script>();
    a->setFetcherForTest([&entered, &left](const QByteArray &, const QByteArray &, const QByteArray &, const std::atomic_bool &stop) {
        entered++;
        while (!stop) QThread::msleep(5);
        left++;
        AutoEqFetchAnswer gone;
        gone.failure = "Cancelled.";
        return gone;
    });
    finished = 0;
    a->startRebuild();
    check("qd27: the fetch was called", waitFor(5000, [&]() { return entered > 0; }));
    QElapsedTimer cancelling;
    cancelling.start();
    a->cancelRebuild();
    const qint64 cancelMs = cancelling.elapsed();
    printf("smoke: qd27 cancel returned in %lld ms\n", static_cast<long long>(cancelMs));
    check("qd27: cancel does not block", cancelMs < 200 && !a->rebuilding());
    check("qd27: cancel reaches the request", waitFor(3000, [&]() { return left == entered.load(); }));
    spin(300);
    check("qd27: cancel reports nothing", finished == 0);
    check("qd27: cancel left the file", fileBytes(db) == kept);

    // Import and reset, off the GUI thread
    const QString good = m_out + "/qd27-good.json", bad = m_out + "/qd27-bad.json";
    { QFile f(good); check("qd27: scratch", f.open(QIODevice::WriteOnly) && f.write(kDatabase) == kDatabase.size()); }
    { QFile f(bad); check("qd27: scratch 2", f.open(QIODevice::WriteOnly) && f.write("{}") == 2); }
    a->importFile(QUrl::fromLocalFile(bad));
    check("qd27: refused import ends", waitFor(5000, [&]() { return updates == 1; }));
    printf("smoke: qd27 refused: %s\n", qPrintable(updateMessage));
    check("qd27: refused import fails, with the file left", !updateOk && !updateMessage.isEmpty() && fileBytes(db) == kept);
    check("qd27: its dialog says so", resultShown("Import Failed", updateMessage));
    a->importFile(QUrl::fromLocalFile(m_out + "/qd27-no-such-file.json"));
    check("qd27: missing file ends", waitFor(5000, [&]() { return updates == 2; }));
    check("qd27: missing file fails", !updateOk && updateMessage == "The file could not be opened.");
    check("qd27: its dialog says that", resultShown("Import Failed", "could not be opened"));
    a->importFile(QUrl::fromLocalFile(good));
    a->resetToBuiltIn();     // Refused: one at a time
    check("qd27: import runs, a second update refused", waitFor(5000, [&]() { return updates == 4; }));
    check("qd27: imported", a->entryCount() == 1 && a->hasUserDatabase() && fileBytes(db) == kDatabase);
    check("qd27: says so", updateOk && updateMessage.contains("Entries: 1"));
    check("qd27: its dialog", resultShown("AutoEQ Database", "Entries: 1"));
    a->resetToBuiltIn();
    check("qd27: reset ends", waitFor(5000, [&]() { return updates == 5; }));
    check("qd27: reset", updateOk && updateMessage.contains(QString("Entries: %1").arg(builtIn)));
    check("qd27: back on the built-in database", a->entryCount() == builtIn && !a->hasUserDatabase() && !QFile::exists(db));
    check("qd27: its dialog says so", resultShown("AutoEQ Database", "Reset to built-in"));

    a->setFetcherForTest(&AutoEqLibrary::httpGet);
    disconnect(rebuilt);
    disconnect(updated);
}

// The real fetch: Qt's network stack on threads the core started
void SmokeDriver::testAutoEqHttp() {
    section("AutoEQ fetch (QD27)");
    auto *a = m_.autoeq;
    const auto closeResult = [this]() {
        spin(300);
        for (QObject *o : root()->findChildren<QObject *>())
            if (o->inherits("QQuickPopup") && o->property("visible").toBool() && o->property("title").toString() == "AutoEQ Database")
                QMetaObject::invokeMethod(o, "close");
        spin(300);
    };

    LoopServer one([](const QByteArray &path) -> QByteArray {
        if (path == "/limit")
            return http("403 Forbidden", "Retry-After: 1200\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 1800000000\r\n", "limit");
        if (path == "/error")
            return http("500 Internal Server Error", "Retry-After: Wed, 21 Oct 2026 07:28:00 GMT\r\n", "boom");
        if (path == "/moved") return http("302 Found", "Location: /ok\r\n", "");
        if (path == "/ok") return http("200 OK", "", "body");
        if (path == "/cut") return "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort";
        return QByteArray();    // /hang
    });
    // On a thread Qt did not start, as the core's are, with this one still free
    const auto get = [&](const char *path, const QByteArray &accept, const std::atomic_bool &stop) {
        AutoEqFetchAnswer got;
        std::atomic_bool done{ false };
        std::thread t([&]() {
            got = AutoEqLibrary::httpGet(one.base() + path, accept, "qd27-agent", stop);
            done = true;
        });
        check("qd27: the fetch returns", waitFor(10000, [&]() { return done.load(); }));
        t.join();
        return got;
    };
    const std::atomic_bool go{ false };

    AutoEqFetchAnswer r = get("/limit", "application/x-test", go);
    check("qd27: a 403 is an answer", r.answered && r.status == 403 && r.body == "limit");
    check("qd27: rate-limit headers read", r.retryAfterS == 1200 && r.rateLimitRemaining == 0 && r.rateLimitReset == 1800000000);
    const QVector<LoopServer::Request> asked = one.requests();
    check("qd27: User-Agent and Accept are sent", asked.size() == 1 && asked[0].agent == "qd27-agent" && asked[0].accept == "application/x-test");

    r = get("/error", QByteArray(), go);
    check("qd27: a 500 is an answer, with its body", r.answered && r.status == 500 && r.body == "boom");
    check("qd27: a date in Retry-After is no number", r.retryAfterS == -1 && r.rateLimitRemaining == -1 && r.rateLimitReset == -1);
    check("qd27: no Accept unless asked", one.requests().value(1).accept.isEmpty());

    r = get("/moved", QByteArray(), go);
    check("qd27: redirects are followed", r.answered && r.status == 200 && r.body == "body");

    r = get("/cut", QByteArray(), go);
    printf("smoke: qd27 cut: %s\n", qPrintable(r.failure));
    check("qd27: a body cut short is no answer", !r.answered && !r.failure.isEmpty());

    {
        std::atomic_bool stop{ false };
        std::thread stopper([&stop]() { QThread::msleep(300); stop = true; });
        QElapsedTimer t;
        t.start();
        r = get("/hang", QByteArray(), stop);
        stopper.join();
        check("qd27: a stop ends a request in flight", !r.answered && r.failure == "Cancelled." && t.elapsed() < 5000);
    }

    QByteArray closedPort;
    {
        QTcpServer gone;
        gone.listen(QHostAddress::LocalHost);
        closedPort = "http://127.0.0.1:" + QByteArray::number(int(gone.serverPort())) + "/";
    }
    r = AutoEqLibrary::httpGet(closedPort, QByteArray(), "qd27-agent", go);
    check("qd27: nothing listening is no answer", !r.answered && !r.failure.isEmpty());

    // A whole rebuild over it: more profiles than one batch, on the core's threads,
    // and all of it done with the GUI thread held
    const int kHeadphones = 25;
    const int kExpected = kHeadphones + 6;  // two listings, four sources with nothing, the profiles
    LoopServer github([&](const QByteArray &path) -> QByteArray {
        if (path.endsWith("/results/oratory1990")) return http("200 OK", "", listing({ "t" }));
        if (path.endsWith("/results/oratory1990/t")) {
            QList<QByteArray> names;
            for (int i = 0; i < kHeadphones; i++) names << "Sony H" + QByteArray::number(i);
            return http("200 OK", "", listing(names));
        }
        if (path.contains("ParametricEQ")) return http("200 OK", "", kProfile);
        return http("404 Not Found", "", "");
    });
    std::atomic_int notRewritten{ 0 };
    const auto toLoopback = [&notRewritten](const QByteArray &base, QByteArray url) {
        if (url.startsWith("https://api.github.com/")) url.replace("https://api.github.com", base);
        else if (url.startsWith("https://raw.githubusercontent.com/")) url.replace("https://raw.githubusercontent.com", base);
        else { notRewritten++; return QByteArray("http://127.0.0.1:1/"); }
        return url;
    };
    const QByteArray base = github.base();
    a->setFetcherForTest([&toLoopback, base](const QByteArray &url, const QByteArray &accept, const QByteArray &agent, const std::atomic_bool &stop) {
        return AutoEqLibrary::httpGet(toLoopback(base, url), accept, agent, stop);
    });
    int finished = 0;
    bool finishedOk = false;
    QString message;
    const QMetaObject::Connection rebuilt = connect(a, &AutoEqLibrary::rebuildFinished, this, [&](bool ok, const QString &m) {
        finished++;
        finishedOk = ok;
        message = m;
    });
    a->startRebuild();
    QElapsedTimer held;
    held.start();
    while (github.served() < kExpected && held.elapsed() < 20000) QThread::msleep(10);
    printf("smoke: qd27 served %d of %d with the GUI thread held for %lld ms\n", github.served(), kExpected, static_cast<long long>(held.elapsed()));
    check("qd27: the rebuild needs nothing of the GUI thread", github.served() >= kExpected);
    check("qd27: rebuild ends", waitFor(15000, [&]() { return finished == 1; }));
    printf("smoke: qd27 real fetch: %s\n", qPrintable(message));
    check("qd27: rebuild over the real fetch", finishedOk && a->entryCount() == kHeadphones);
    check("qd27: no URL outside GitHub's, none left unrewritten", notRewritten == 0);
    bool agents = !github.requests().isEmpty();
    for (const LoopServer::Request &q : github.requests()) agents = agents && q.agent == "DSPi-Console";
    check("qd27: every request carried the User-Agent", agents);
    closeResult();

    // Cancel with requests really in flight
    LoopServer silent([](const QByteArray &) { return QByteArray(); });
    const QByteArray silentBase = silent.base();
    std::atomic_int in{ 0 }, out{ 0 };
    a->setFetcherForTest([&, silentBase](const QByteArray &url, const QByteArray &accept, const QByteArray &agent, const std::atomic_bool &stop) {
        in++;
        const AutoEqFetchAnswer got = AutoEqLibrary::httpGet(toLoopback(silentBase, url), accept, agent, stop);
        out++;
        return got;
    });
    finished = 0;
    a->startRebuild();
    check("qd27: a request hangs", waitFor(5000, [&]() { return silent.served() > 0; }));
    QElapsedTimer cancelling;
    cancelling.start();
    a->cancelRebuild();
    check("qd27: cancel does not wait for it", cancelling.elapsed() < 200 && !a->rebuilding());
    check("qd27: and it ends well before its timeout", waitFor(3000, [&]() { return in > 0 && out == in.load(); }));
    // The core may ask once more before it sees the stop, and only that one
    spin(300);
    check("qd27: cancelled rebuild reports nothing", finished == 0);
    a->setFetcherForTest(&AutoEqLibrary::httpGet);
    disconnect(rebuilt);

    // Leave what the checks after this expect
    QEventLoop waiting;
    const QMetaObject::Connection reset = connect(a, &AutoEqLibrary::updateFinished, &waiting, &QEventLoop::quit);
    a->resetToBuiltIn();
    waiting.exec();
    disconnect(reset);
    closeResult();
    check("qd27: built-in again", !a->hasUserDatabase());
}
