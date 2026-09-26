import LocalRouterKit
import SwiftUI

struct HelpView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text("Give a coding agent access").font(.headline)
                Text("Install the command line tool from the ⋯ menu, then run:")
                CopyLine(text: model.mcpCommand)
                Text("Then ask the agent, in your project folder, to set up LocalRouter:")
                CopyLine(text: model.agentPrompt)
                Button("Open \(model.agentHelpURL)") { model.open(model.agentHelpURL) }
                    .buttonStyle(.link)
                Text("The page lists the steps, the current routes and whether HTTP, HTTPS and the CA work.")
                    .font(.caption).foregroundStyle(.secondary)

                Text("Add routes from a terminal").font(.headline).padding(.top, 6)
                CopyLine(text: "localrouter add shop 5173")
                CopyLine(text: "localrouter add feat-login.shop 5174")
                CopyLine(text: "localrouter add db.shop 55001 --tcp --listen 15432")
                Text("HTTP routes: https://shop.localhost. TCP routes: db.shop.localhost:15432.")
                    .font(.caption).foregroundStyle(.secondary)

                Text("HTTPS outside Safari and Chrome").font(.headline).padding(.top, 6)
                Text("Firefox: open about:config and set security.enterprise_roots.enabled to true.")
                Text("Node.js:")
                CopyLine(text: "export NODE_EXTRA_CA_CERTS=\"$(localrouter ca-path)\"")
                Text("Python requests:")
                CopyLine(text: "export REQUESTS_CA_BUNDLE=\"$(localrouter ca-path)\"")

                Text("Why .localhost").font(.headline).padding(.top, 6)
                Text("macOS resolves every *.localhost name to this Mac by itself, so LocalRouter needs no DNS server and no admin rights for names.")
                    .foregroundStyle(.secondary)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
