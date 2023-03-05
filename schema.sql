DROP TABLE IF EXISTS Users;
CREATE TABLE Users(
    Id INTEGER PRIMARY KEY,
    Username TEXT NOT NULL,
    Email TEXT NOT NULL, 
    LastSynced INTEGER
);

DROP TABLE IF EXISTS UserSessions;
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

DROP TABLE IF EXISTS Campaigns;
CREATE TABLE Campaigns(
    Id TEXT PRIMARY KEY,
    Title TEXT NOT NULL,
    MemberListId TEXT NOT NULL,
    UserId INTEGER NOT NULL,
    FOREIGN KEY (UserId)
        REFERENCES Users (Id)
            ON UPDATE CASCADE
            ON DELETE CASCADE
);

DROP TABLE IF EXISTS Members;
CREATE TABLE Members(
    EmailId TEXT NOT NULL,
    FullName TEXT NOT NULL,
    CampaignId TEXT NOT NULL,
    FOREIGN KEY (CampaignId)
        REFERENCES Campaigns (Id)
            ON UPDATE CASCADE
            ON DELETE CASCADE
);