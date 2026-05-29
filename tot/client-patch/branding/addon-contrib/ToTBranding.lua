-- ToTBranding.lua — login-screen version watermark + mandatory fan-project
-- disclaimer (design §5.2, spec §10.4). @TOT_VERSION@ is substituted at
-- compose/pack time by pack-mpq.py. AUTO-COMPOSED into the ThreadsOfTime AddOn.
--
-- PRE-FLIGHT (verify on the real 3.3.5a client before shipping):
--   1. The glue version FontString name. `VersionLabel` is the common 3.3.5a
--      name; if absent, this hook no-ops harmlessly. Confirm on the client.
--   2. Whether glue-screen AddOns load on this build. If they do not, ship the
--      branding via the FrameXML-override rail instead (design §5.3).

ThreadsOfTime = ThreadsOfTime or {}
ThreadsOfTime.Branding = ThreadsOfTime.Branding or {}

local TOT_VERSION = "@TOT_VERSION@"
local DISCLAIMER = "Unofficial non-commercial fan project. Not affiliated with"
    .. " Blizzard Entertainment. WoW and Wrath of the Lich King are trademarks"
    .. " of Blizzard Entertainment."

local function applyBranding()
    if _G.VersionLabel and _G.VersionLabel.SetText then
        _G.VersionLabel:SetText(
            "Threads of Time " .. TOT_VERSION .. "\n|cff888888" .. DISCLAIMER .. "|r")
    end
end

applyBranding()
if _G.GlueParent and _G.GlueParent.HookScript then
    _G.GlueParent:HookScript("OnShow", applyBranding)
end
