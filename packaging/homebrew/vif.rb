# The cask for the Pisosenie/homebrew-tap repository: `brew install --cask pisosenie/tap/vif`.
cask "vif" do
  version "0.1.4"
  sha256 "01b488655b7ff521726ac89e2ed9b64165e19e4c1692a0d1edae4ac0663d2ff5"

  url "https://github.com/Pisosenie/vif/releases/download/v#{version}/vif-#{version}-macos.zip"
  name "vif"
  desc "Fast GPU terminal with command blocks, sessions and themes"
  homepage "https://github.com/Pisosenie/vif"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: :ventura

  app "vif.app"
  binary "#{appdir}/vif.app/Contents/MacOS/vif"
  binary "#{appdir}/vif.app/Contents/MacOS/vif-settings"

  zap trash: [
    "~/.config/vif",
    "~/.local/share/vif",
    "~/Library/Saved Application State/io.github.pisosenie.vif.savedState",
  ]

  caveats <<~EOS
    vif isn't notarized yet, so macOS refuses it the first time. Open it once with
    System Settings → Privacy & Security → Open Anyway, or run:
      xattr -dr com.apple.quarantine #{appdir}/vif.app
  EOS
end
