// SPDX-License-Identifier: GPL-3.0-or-later
//
// Redrob Launcher's shared account file as this app reads it (redrob-launcher
// docs/shared-account.md). REDROB_SHARED_DIR points the reader at a folder this test owns, so the
// sign-in of the machine running the tests is neither read nor touched.
#include "../SharedAccount.h"

#include <QDir>
#include <QFile>
#include <QTemporaryDir>
#include <QtTest>

namespace {
bool writeFile(const QString &path, const QByteArray &bytes)
{
    QDir().mkpath(QFileInfo(path).absolutePath());
    QFile file(path);
    if (!file.open(QIODevice::WriteOnly | QIODevice::Truncate))
        return false;
    return file.write(bytes) == bytes.size();
}
} // namespace

class SharedAccountTest : public QObject
{
    Q_OBJECT

private slots:
    void init()
    {
        QVERIFY(m_dir.isValid());
        qputenv("REDROB_SHARED_DIR", m_dir.path().toUtf8());
        QFile::remove(SharedAccount::filePath());
    }

    void cleanup() { qunsetenv("REDROB_SHARED_DIR"); }

    void redrob_shared_dir_replaces_the_directory()
    {
        QCOMPARE(SharedAccount::filePath(),
                 QDir(m_dir.path()).filePath(QStringLiteral("account.json")));
    }

    void the_platform_default_is_used_when_the_override_is_empty()
    {
        qputenv("REDROB_SHARED_DIR", "");
        const QString path = SharedAccount::filePath();
        QVERIFY(!path.isEmpty());
        QVERIFY(path.endsWith(QStringLiteral("account.json")));
#if defined(Q_OS_WIN)
        QVERIFY(path.contains(QStringLiteral("Redrob/Shared/")));
#elif defined(Q_OS_MACOS)
        QVERIFY(path.contains(QStringLiteral("Library/Application Support/ai.redrob.shared/")));
#else
        QVERIFY(path.contains(QStringLiteral("redrob-shared/")));
        // $XDG_CONFIG_HOME replaces ~/.config.
        const QByteArray previous = qgetenv("XDG_CONFIG_HOME");
        qputenv("XDG_CONFIG_HOME", m_dir.path().toUtf8());
        QCOMPARE(SharedAccount::filePath(),
                 QDir(m_dir.path()).filePath(QStringLiteral("redrob-shared/account.json")));
        if (previous.isEmpty())
            qunsetenv("XDG_CONFIG_HOME");
        else
            qputenv("XDG_CONFIG_HOME", previous);
#endif
    }

    void a_launcher_file_is_a_sign_in_with_names_only()
    {
        QVERIFY(writeFile(SharedAccount::filePath(),
                          "{\"apiKey\": \"rrk_test_secret\", \"apiKeyName\": \"Redrob Launcher\","
                          " \"accountName\": \"Acme\","
                          " \"apiBaseUrl\": \"https://console.redrob.ai/api/backend/v1\"}"));
        const SharedAccount account = SharedAccount::read();
        QVERIFY(account.signedIn());
        QCOMPARE(account.apiKey, QByteArray("rrk_test_secret"));
        QCOMPARE(account.apiKeyName, QStringLiteral("Redrob Launcher"));
        QCOMPARE(account.accountName, QStringLiteral("Acme"));
        QCOMPARE(account.apiBaseUrl, QStringLiteral("https://console.redrob.ai/api/backend/v1"));
    }

    void missing_malformed_and_empty_all_mean_not_signed_in()
    {
        QVERIFY(!QFile::exists(SharedAccount::filePath()));
        QVERIFY(!SharedAccount::read().signedIn());

        QVERIFY(writeFile(SharedAccount::filePath(), "{not json"));
        QVERIFY(!SharedAccount::read().signedIn());

        QVERIFY(writeFile(SharedAccount::filePath(), "[\"rrk_in_an_array\"]"));
        QVERIFY(!SharedAccount::read().signedIn());

        QVERIFY(writeFile(SharedAccount::filePath(), "{\"apiKey\": \"\", \"accountName\": \"Acme\"}"));
        QVERIFY(!SharedAccount::read().signedIn());

        QVERIFY(writeFile(SharedAccount::filePath(), "{\"apiKey\": \"   \"}"));
        QVERIFY(!SharedAccount::read().signedIn());

        QVERIFY(writeFile(SharedAccount::filePath(), "{\"apiKey\": 42}"));
        QVERIFY(!SharedAccount::read().signedIn());

        QVERIFY(writeFile(SharedAccount::filePath(), "{\"accountName\": \"Acme\"}"));
        QVERIFY(!SharedAccount::read().signedIn());

        QVERIFY(!SharedAccount::readFrom(QString{}).signedIn());
    }

    void the_file_is_read_fresh_each_time()
    {
        QVERIFY(writeFile(SharedAccount::filePath(), "{\"apiKey\": \"rrk_first\"}"));
        QCOMPARE(SharedAccount::read().apiKey, QByteArray("rrk_first"));
        QVERIFY(writeFile(SharedAccount::filePath(), "{\"apiKey\": \"rrk_second\"}"));
        QCOMPARE(SharedAccount::read().apiKey, QByteArray("rrk_second"));
        QVERIFY(QFile::remove(SharedAccount::filePath()));
        QVERIFY(!SharedAccount::read().signedIn());
    }

    void reading_never_creates_or_changes_the_file()
    {
        QVERIFY(!SharedAccount::read().signedIn());
        QVERIFY(!QFile::exists(SharedAccount::filePath()));
        const QByteArray bytes = "{\"apiKey\": \"rrk_keep\", \"accountName\": \"Acme\"}";
        QVERIFY(writeFile(SharedAccount::filePath(), bytes));
        (void)SharedAccount::read();
        QFile file(SharedAccount::filePath());
        QVERIFY(file.open(QIODevice::ReadOnly));
        QCOMPARE(file.readAll(), bytes);
    }

private:
    QTemporaryDir m_dir;
};

QTEST_GUILESS_MAIN(SharedAccountTest)
#include "tst_shared_account.moc"
