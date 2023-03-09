DROP TABLE IF EXISTS Campaigns;
DROP TABLE IF EXISTS Members;
DROP TABLE IF EXISTS Lists;
DROP TABLE IF EXISTS UserSessions;
DROP TABLE IF EXISTS Users;

CREATE TABLE Users(
    Id INTEGER PRIMARY KEY,
    Username TEXT NOT NULL,
    Email TEXT NOT NULL
);

CREATE TABLE UserSessions(
    Id TEXT PRIMARY KEY,
    UserId INTEGER NOT NULL,
    AccessToken TEXT NOT NULL,
    Dc TEXT NOT NULL,
    FOREIGN KEY (UserId)
        REFERENCES Users (Id)
            ON UPDATE CASCADE
            ON DELETE CASCADE
);

CREATE TABLE Lists(
    Id TEXT PRIMARY KEY,
    UserId INTEGER NOT NULL,
    WebhookId TEXT NOT NULL,
    FOREIGN KEY (UserId)
        REFERENCES Users (Id)
            ON UPDATE CASCADE
            ON DELETE CASCADE
);

CREATE TABLE Campaigns(
    Id TEXT PRIMARY KEY,
    Title TEXT NOT NULL,
    ListId TEXT NOT NULL,
    UserId INTEGER NOT NULL,
    VideoTag TEXT NOT NULL,
    ImageTag TEXT NOT NULL,
    FOREIGN KEY (ListId)
        REFERENCES Lists (Id)
            ON UPDATE CASCADE
            ON DELETE CASCADE,
    FOREIGN KEY (UserId)
        REFERENCES Users (Id)
            ON UPDATE CASCADE
            ON DELETE CASCADE
);

CREATE TABLE Members(
    EmailId TEXT NOT NULL,
    FullName TEXT NOT NULL,
    ListId TEXT NOT NULL,
    FOREIGN KEY (ListId)
        REFERENCES Lists (Id)
            ON UPDATE CASCADE
            ON DELETE CASCADE
);