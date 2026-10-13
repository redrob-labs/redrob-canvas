// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QByteArray>
#include <QString>

// Redrob Launcher's shared account file: the workspace this computer is signed in to, which every
// Redrob app may read and only the launcher writes (redrob-launcher docs/shared-account.md).
//
//   Linux    $XDG_CONFIG_HOME/redrob-shared/account.json   (~/.config/redrob-shared/account.json)
//   macOS    ~/Library/Application Support/ai.redrob.shared/account.json
//   Windows  %APPDATA%\Redrob\Shared\account.json
//
// REDROB_SHARED_DIR, when set and not empty, replaces the directory on every platform.
//
// Rules for a reader, all of which this type follows: read, never write; read it when the key is
// needed rather than once at startup; a missing file, unreadable JSON or an empty apiKey all mean
// "not signed in"; never log or display apiKey -- show accountName and apiKeyName only.
struct SharedAccount
{
    QByteArray apiKey;
    QString apiKeyName;
    QString accountName;
    QString apiBaseUrl;

    bool signedIn() const { return !apiKey.isEmpty(); }

    // Where the file is on this computer; empty when no home or profile folder is known.
    static QString filePath();

    // A fresh read of the file at filePath(). Anything short of a JSON object with a non-empty
    // string apiKey yields a SharedAccount that is not signed in.
    static SharedAccount read();

    // The same parse for a given file; filePath() is what the app uses, this is for the tests.
    static SharedAccount readFrom(const QString &path);
};
