--[[
    mod-warforged client-side AddonMessage responder.

    Server-side WarforgedSentinelScript schedules a PING ~3s after login on
    the addon channel (LANG_ADDON whisper, prefix "WF", body "PING:<version>").
    This frame replies with "WF\tPONG:<our patch version>" so the server can
    confirm the client has patch-W.MPQ loaded.

    Shipped inside patch-W.MPQ at Interface\FrameXML\WarforgedSentinel.lua,
    which the WoW 3.3.5a client autoloads after Blizzard FrameXML.
]]

-- WARFORGED_PATCH_VERSION is set by data/lua/GameTooltip.lua at the top of
-- that file. If we're loaded without it (manual install error or load-order
-- skew), fall back to "0.0.0" so the server still gets a parseable PONG.
local function GetPatchVersion()
    return WARFORGED_PATCH_VERSION or "0.0.0"
end

local frame = CreateFrame("Frame", "WarforgedSentinelFrame")
frame:RegisterEvent("CHAT_MSG_ADDON")
frame:SetScript("OnEvent", function(self, event, prefix, message, channel, sender)
    if prefix ~= "WF" then return end
    if type(message) ~= "string" then return end
    if string.sub(message, 1, 5) ~= "PING:" then return end

    -- Whisper-channel back to ourselves; the 3.3.5a client routes the
    -- LANG_ADDON whisper through CHAT_MSG_ADDON on the receiving end.
    SendAddonMessage("WF", "PONG:" .. GetPatchVersion(), "WHISPER", UnitName("player"))
end)
