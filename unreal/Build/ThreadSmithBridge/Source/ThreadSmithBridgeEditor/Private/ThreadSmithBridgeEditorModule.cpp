// Starts the bridge and adds ThreadSmith to the editor:
// - Window > ThreadSmith Wardrobe, and a Wardrobe button on the level toolbar
// - "To ThreadSmith" toolbar button and right-click entries (level + Content Browser)
// - "Send to ThreadSmith" and "Wardrobe" buttons in the Blueprint editor toolbar
//   (like MetaTailor's Send to MT / Receive from MT)

#include "SThreadSmithWardrobe.h"
#include "ThreadSmithBridge.h"

#include "ContentBrowserMenuContexts.h"
#include "Engine/Blueprint.h"
#include "Interfaces/IPluginManager.h"
#include "Styling/SlateStyle.h"
#include "Styling/SlateStyleRegistry.h"
#include "Toolkits/AssetEditorToolkitMenuContext.h"
#include "Engine/SkeletalMesh.h"
#include "Framework/Docking/TabManager.h"
#include "Modules/ModuleManager.h"
#include "Styling/AppStyle.h"
#include "ToolMenus.h"
#include "Widgets/Docking/SDockTab.h"

#define LOCTEXT_NAMESPACE "ThreadSmithBridge"

static const FName WardrobeTabName(TEXT("ThreadSmithWardrobe"));
static const FName StyleName(TEXT("ThreadSmithStyle"));

class FThreadSmithBridgeEditorModule : public IModuleInterface
{
public:
	virtual void StartupModule() override
	{
		FThreadSmithBridge::Get().Start();
		RegisterStyle();

		FGlobalTabmanager::Get()->RegisterNomadTabSpawner(WardrobeTabName, FOnSpawnTab::CreateLambda([](const FSpawnTabArgs&)
		{
			return SNew(SDockTab).TabRole(ETabRole::NomadTab)[SNew(SThreadSmithWardrobe)];
		}))
		.SetDisplayName(LOCTEXT("WardrobeTab", "ThreadSmith Wardrobe"))
		.SetTooltipText(LOCTEXT("WardrobeTabTip", "Outfits from your ThreadSmith library."))
		.SetIcon(FSlateIcon(StyleName, "ThreadSmith.Wardrobe", "ThreadSmith.Wardrobe.Small"))
		.SetMenuType(ETabSpawnerMenuType::Hidden);

		UToolMenus::RegisterStartupCallback(FSimpleMulticastDelegate::FDelegate::CreateRaw(this, &FThreadSmithBridgeEditorModule::RegisterMenus));
	}

	virtual void ShutdownModule() override
	{
		UToolMenus::UnRegisterStartupCallback(this);
		UToolMenus::UnregisterOwner(this);
		if (FSlateApplication::IsInitialized())
		{
			FGlobalTabmanager::Get()->UnregisterNomadTabSpawner(WardrobeTabName);
		}
		FThreadSmithBridge::Get().Stop();
		if (Style.IsValid())
		{
			FSlateStyleRegistry::UnRegisterSlateStyle(*Style);
			Style.Reset();
		}
	}

private:
	TSharedPtr<FSlateStyleSet> Style;

	/** ThreadSmith icons: the logo, Export (send to ThreadSmith), Import (bring
	 *  an outfit in) and Wardrobe. Each has a 40 px, a 20 px ("Small") and a
	 *  16 px ("Button") size. */
	void RegisterStyle()
	{
		TSharedPtr<IPlugin> Plugin = IPluginManager::Get().FindPlugin(TEXT("ThreadSmithBridge"));
		if (!Plugin.IsValid())
		{
			return;
		}
		const FString Res = Plugin->GetBaseDir() / TEXT("Resources");
		Style = MakeShared<FSlateStyleSet>(StyleName);
		Style->SetContentRoot(Res);
		auto AddIcon = [this, &Res](const TCHAR* Key, const TCHAR* File)
		{
			const FString Name(Key);
			Style->Set(*Name, new FSlateImageBrush(Res / FString::Printf(TEXT("%s40.png"), File), FVector2D(40.0, 40.0)));
			Style->Set(*(Name + TEXT(".Small")), new FSlateImageBrush(Res / FString::Printf(TEXT("%s20.png"), File), FVector2D(20.0, 20.0)));
			Style->Set(*(Name + TEXT(".Button")), new FSlateImageBrush(Res / FString::Printf(TEXT("%s20.png"), File), FVector2D(16.0, 16.0)));
		};
		AddIcon(TEXT("ThreadSmith.Icon"), TEXT("ToolbarIcon"));
		AddIcon(TEXT("ThreadSmith.Export"), TEXT("Export"));
		AddIcon(TEXT("ThreadSmith.Import"), TEXT("Import"));
		AddIcon(TEXT("ThreadSmith.Wardrobe"), TEXT("Wardrobe"));
		FSlateStyleRegistry::RegisterSlateStyle(*Style);
	}

	/** The Blueprint open in the Blueprint editor this toolbar belongs to. */
	static UBlueprint* EditedBlueprint(const FToolMenuContext& Context)
	{
		if (const UAssetEditorToolkitMenuContext* Ctx = Context.FindContext<UAssetEditorToolkitMenuContext>())
		{
			for (UObject* Obj : Ctx->GetEditingObjects())
			{
				if (UBlueprint* BP = Cast<UBlueprint>(Obj))
				{
					return BP;
				}
			}
		}
		return nullptr;
	}

	static void OpenWardrobe()
	{
		FGlobalTabmanager::Get()->TryInvokeTab(WardrobeTabName);
	}

	void RegisterMenus()
	{
		FToolMenuOwnerScoped OwnerScoped(this);
		const FSlateIcon WardrobeIcon(StyleName, "ThreadSmith.Wardrobe", "ThreadSmith.Wardrobe.Small");
		const FSlateIcon ExportIcon(StyleName, "ThreadSmith.Export", "ThreadSmith.Export.Small");
		const FUIAction OpenAction(FExecuteAction::CreateStatic(&FThreadSmithBridgeEditorModule::OpenWardrobe));
		const FUIAction SendAction(FExecuteAction::CreateLambda([] { FThreadSmithBridge::Get().SendSelectionToThreadSmith(); }));

		// Window menu
		if (UToolMenu* Window = UToolMenus::Get()->ExtendMenu("LevelEditor.MainMenu.Window"))
		{
			FToolMenuSection& Section = Window->FindOrAddSection("ThreadSmith", LOCTEXT("ThreadSmith", "ThreadSmith"));
			Section.AddMenuEntry("ThreadSmithWardrobe", LOCTEXT("Wardrobe", "ThreadSmith Wardrobe"),
				LOCTEXT("WardrobeTip", "Open the ThreadSmith Wardrobe."), WardrobeIcon, OpenAction);
		}

		// Level editor toolbar
		if (UToolMenu* Toolbar = UToolMenus::Get()->ExtendMenu("LevelEditor.LevelEditorToolBar.User"))
		{
			FToolMenuSection& Section = Toolbar->FindOrAddSection("ThreadSmith");
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithOpenWardrobe", OpenAction,
				LOCTEXT("WardrobeButton", "Wardrobe"), LOCTEXT("WardrobeTip", "Open the ThreadSmith Wardrobe."), WardrobeIcon));
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithSendBody", SendAction,
				LOCTEXT("SendButton", "To ThreadSmith"), LOCTEXT("SendTip", "Send the selected character's body to ThreadSmith."), ExportIcon));
		}

		// Blueprint editor toolbar (like MetaTailor's Send to MT / Receive from MT)
		if (UToolMenu* BPToolbar = UToolMenus::Get()->ExtendMenu("AssetEditor.BlueprintEditor.ToolBar"))
		{
			FToolMenuSection& Section = BPToolbar->FindOrAddSection("ThreadSmith");
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithSendBlueprint",
				FToolUIActionChoice(FToolMenuExecuteAction::CreateLambda([](const FToolMenuContext& Context)
				{
					if (UBlueprint* BP = EditedBlueprint(Context))
					{
						FThreadSmithBridge::Get().SendBlueprintToThreadSmith(BP);
					}
				})),
				LOCTEXT("SendBP", "Send to ThreadSmith"),
				LOCTEXT("SendBPTip", "Send this character's body and head to ThreadSmith. Outfits you send back are added to this Blueprint."),
				ExportIcon));
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithBPWardrobe", OpenAction,
				LOCTEXT("BPWardrobe", "ThreadSmith Wardrobe"),
				LOCTEXT("BPWardrobeTip", "Open the Wardrobe to add outfits to this Blueprint. Dock it next to the Blueprint editor."),
				WardrobeIcon));
		}

		// Right-click on an actor in the level
		if (UToolMenu* ActorMenu = UToolMenus::Get()->ExtendMenu("LevelEditor.ActorContextMenu"))
		{
			FToolMenuSection& Section = ActorMenu->FindOrAddSection("ThreadSmith", LOCTEXT("ThreadSmith", "ThreadSmith"));
			Section.AddMenuEntry("ThreadSmithSendActor", LOCTEXT("SendActor", "Send to ThreadSmith"),
				LOCTEXT("SendActorTip", "Export this character's body and open it in ThreadSmith."), ExportIcon, SendAction);
		}

		// Right-click on a skeletal mesh in the Content Browser
		if (UToolMenu* AssetMenu = UToolMenus::Get()->ExtendMenu("ContentBrowser.AssetContextMenu.SkeletalMesh"))
		{
			FToolMenuSection& Section = AssetMenu->FindOrAddSection("ThreadSmith", LOCTEXT("ThreadSmith", "ThreadSmith"));
			Section.AddMenuEntry("ThreadSmithSendAsset", LOCTEXT("SendAsset", "Send to ThreadSmith"),
				LOCTEXT("SendAssetTip", "Export this skeletal mesh and open it in ThreadSmith as the body."), ExportIcon,
				FToolUIActionChoice(FToolMenuExecuteAction::CreateLambda([](const FToolMenuContext& Context)
				{
					const UContentBrowserAssetContextMenuContext* Ctx = Context.FindContext<UContentBrowserAssetContextMenuContext>();
					if (!Ctx)
					{
						return;
					}
					for (const FAssetData& Asset : Ctx->SelectedAssets)
					{
						if (USkeletalMesh* Mesh = Cast<USkeletalMesh>(Asset.GetAsset()))
						{
							FThreadSmithBridge::Get().SendBodyToThreadSmith(Mesh, nullptr);
							return;
						}
					}
				})));
		}
	}
};

IMPLEMENT_MODULE(FThreadSmithBridgeEditorModule, ThreadSmithBridgeEditor)

#undef LOCTEXT_NAMESPACE
