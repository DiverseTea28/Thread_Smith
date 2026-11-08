#include "ThreadSmithBridge.h"

#include "GameFramework/Character.h"
#include "Engine/Blueprint.h"
#include "Engine/SCS_Node.h"
#include "Engine/SimpleConstructionScript.h"
#include "Kismet2/BlueprintEditorUtils.h"
#include "Kismet2/KismetEditorUtilities.h"
#include "Subsystems/AssetEditorSubsystem.h"
#include "ThreadSmithBridgeSettings.h"
#include "ThreadSmithLibrary.h"
#include "ThreadSmithOutfitComponent.h"

#include "AssetExportTask.h"
#include "AssetImportTask.h"
#include "AssetRegistry/AssetRegistryModule.h"
#include "AssetToolsModule.h"
#include "Components/SkeletalMeshComponent.h"
#include "Dom/JsonObject.h"
#include "Editor.h"
#include "Engine/Selection.h"
#include "Engine/SkeletalMesh.h"
#include "Exporters/Exporter.h"
#include "Exporters/FbxExportOption.h"
#include "EngineUtils.h"
#include "FileHelpers.h"
#include "Framework/Notifications/NotificationManager.h"
#include "GameFramework/Actor.h"
#include "HAL/FileManager.h"
#include "HAL/PlatformProcess.h"
#include "HttpModule.h"
#include "HttpPath.h"
#include "HttpServerModule.h"
#include "HttpServerResponse.h"
#include "IHttpRouter.h"
#include "Interfaces/IHttpRequest.h"
#include "Interfaces/IHttpResponse.h"
#include "InterchangeGenericAnimationPipeline.h"
#include "InterchangeGenericAssetsPipeline.h"
#include "InterchangeGenericAssetsPipelineSharedSettings.h"
#include "InterchangeGenericMeshPipeline.h"
#include "InterchangeManager.h"
#include "Misc/EngineVersion.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "ObjectTools.h"
#include "ScopedTransaction.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"
#include "Serialization/JsonWriter.h"
#include "Widgets/Notifications/SNotificationList.h"

DEFINE_LOG_CATEGORY(LogThreadSmith);

#define LOCTEXT_NAMESPACE "ThreadSmithBridge"

namespace
{
	constexpr int32 ProtocolVersion = 1;

	void Notify(const FText& Text, bool bSuccess)
	{
		FNotificationInfo Info(Text);
		Info.ExpireDuration = bSuccess ? 4.0f : 8.0f;
		Info.bUseLargeFont = false;
		if (TSharedPtr<SNotificationItem> Item = FSlateNotificationManager::Get().AddNotification(Info))
		{
			Item->SetCompletionState(bSuccess ? SNotificationItem::CS_Success : SNotificationItem::CS_Fail);
		}
	}

	FString ToJson(const TSharedRef<FJsonObject>& Obj)
	{
		FString Out;
		TSharedRef<TJsonWriter<>> Writer = TJsonWriterFactory<>::Create(&Out);
		FJsonSerializer::Serialize(Obj, Writer);
		return Out;
	}

	TUniquePtr<FHttpServerResponse> JsonResponse(const TSharedRef<FJsonObject>& Obj, EHttpServerResponseCodes Code = EHttpServerResponseCodes::Ok)
	{
		TUniquePtr<FHttpServerResponse> R = FHttpServerResponse::Create(ToJson(Obj), TEXT("application/json"));
		R->Code = Code;
		return R;
	}

	TUniquePtr<FHttpServerResponse> ErrorResponse(const FString& Message, EHttpServerResponseCodes Code = EHttpServerResponseCodes::BadRequest)
	{
		TSharedRef<FJsonObject> Obj = MakeShared<FJsonObject>();
		Obj->SetBoolField(TEXT("ok"), false);
		Obj->SetStringField(TEXT("error"), Message);
		return JsonResponse(Obj, Code);
	}

	FString BodyAsString(const FHttpServerRequest& Request)
	{
		FUTF8ToTCHAR Conv(reinterpret_cast<const ANSICHAR*>(Request.Body.GetData()), Request.Body.Num());
		return FString(Conv.Length(), Conv.Get());
	}

	const UThreadSmithBridgeSettings* Settings()
	{
		return GetDefault<UThreadSmithBridgeSettings>();
	}

	/** Saved/ThreadSmith/trimmed_bodies.json: trimmed body asset path -> the original body path. */
	FString TrimmedMapFile()
	{
		return FPaths::ConvertRelativePathToFull(FPaths::ProjectSavedDir() / TEXT("ThreadSmith/trimmed_bodies.json"));
	}

	TSharedPtr<FJsonObject> LoadTrimmedMap()
	{
		FString Text;
		TSharedPtr<FJsonObject> Map;
		if (!FFileHelper::LoadFileToString(Text, *TrimmedMapFile())
			|| !FJsonSerializer::Deserialize(TJsonReaderFactory<>::Create(Text), Map) || !Map.IsValid())
		{
			Map = MakeShared<FJsonObject>();
		}
		return Map;
	}

	void RememberOriginalBody(USkeletalMesh* Trimmed, USkeletalMesh* Original)
	{
		if (!Trimmed || !Original || Trimmed == Original)
		{
			return;
		}
		TSharedPtr<FJsonObject> Map = LoadTrimmedMap();
		// If the "original" is itself a trimmed body, point at its original instead.
		FString OriginalPath = Original->GetPathName();
		Map->TryGetStringField(OriginalPath, OriginalPath);
		Map->SetStringField(Trimmed->GetPathName(), OriginalPath);
		FFileHelper::SaveStringToFile(ToJson(Map.ToSharedRef()), *TrimmedMapFile());
	}

	/** The full body for a body that ThreadSmith trimmed (or the mesh itself). */
	USkeletalMesh* UntrimmedBody(USkeletalMesh* Mesh)
	{
		FString OriginalPath;
		if (Mesh && LoadTrimmedMap()->TryGetStringField(Mesh->GetPathName(), OriginalPath))
		{
			if (USkeletalMesh* Original = LoadObject<USkeletalMesh>(nullptr, *OriginalPath, nullptr, LOAD_NoWarn | LOAD_Quiet))
			{
				return Original;
			}
		}
		return Mesh;
	}

	/** Import a GLB as a skeletal mesh on an existing skeleton (Interchange, tested in UE 5.8.3). */
	USkeletalMesh* ImportGlbOntoSkeleton(const FString& Glb, const FString& AssetName, const FString& Folder, USkeleton* Skeleton, FString& OutError)
	{
		// Copy the GLB to a file with the asset's name, so the asset gets that name.
		const FString TempDir = FPaths::ConvertRelativePathToFull(FPaths::ProjectSavedDir() / TEXT("ThreadSmith/Import"));
		IFileManager::Get().MakeDirectory(*TempDir, true);
		const FString TempFile = TempDir / (AssetName + TEXT(".glb"));
		if (IFileManager::Get().Copy(*TempFile, *Glb, /*Replace*/ true, /*EvenIfReadOnly*/ true) != COPY_OK)
		{
			OutError = FString::Printf(TEXT("couldn't read %s"), *Glb);
			return nullptr;
		}

		UInterchangeGenericAssetsPipeline* Pipeline = NewObject<UInterchangeGenericAssetsPipeline>(GetTransientPackage());
		Pipeline->CommonSkeletalMeshesAndAnimationsProperties->Skeleton = Skeleton;
		Pipeline->CommonSkeletalMeshesAndAnimationsProperties->bImportOnlyAnimations = false;
		Pipeline->MeshPipeline->bImportSkeletalMeshes = true;
		Pipeline->MeshPipeline->bImportStaticMeshes = false;
		Pipeline->AnimationPipeline->bImportAnimations = false;

		UInterchangePipelineStackOverride* Stack = NewObject<UInterchangePipelineStackOverride>(GetTransientPackage());
		Stack->OverridePipelines.Add(FSoftObjectPath(Pipeline));

		UAssetImportTask* Task = NewObject<UAssetImportTask>();
		Task->Filename = TempFile;
		Task->DestinationPath = Folder;
		Task->bAutomated = true;
		Task->bReplaceExisting = true;
		Task->bSave = true;
		Task->Options = Stack;

		FAssetToolsModule::GetModule().Get().ImportAssetTasks({ Task });

		for (const FString& Path : Task->ImportedObjectPaths)
		{
			if (USkeletalMesh* Mesh = Cast<USkeletalMesh>(StaticLoadObject(UObject::StaticClass(), nullptr, *Path)))
			{
				if (Mesh->GetSkeleton() != Skeleton)
				{
					UE_LOG(LogThreadSmith, Warning, TEXT("%s was imported onto %s instead of %s"), *Path,
						Mesh->GetSkeleton() ? *Mesh->GetSkeleton()->GetPathName() : TEXT("nothing"), *Skeleton->GetPathName());
				}
				return Mesh;
			}
		}
		OutError = TEXT("the importer didn't create a skeletal mesh");
		return nullptr;
	}
}

FThreadSmithBridge& FThreadSmithBridge::Get()
{
	static FThreadSmithBridge Instance;
	return Instance;
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

void FThreadSmithBridge::Start()
{
	FHttpServerModule& Http = FHttpServerModule::Get();
	Router = Http.GetHttpRouter(Settings()->PluginPort, /*bFailOnBindFailure*/ true);
	if (!Router.IsValid())
	{
		UE_LOG(LogThreadSmith, Error, TEXT("Couldn't listen on port %d (another editor open?). Outfits from ThreadSmith won't arrive."), Settings()->PluginPort);
		return;
	}
	Routes.Add(Router->BindRoute(FHttpPath(TEXT("/status")), EHttpServerRequestVerbs::VERB_GET,
		FHttpRequestHandler::CreateRaw(this, &FThreadSmithBridge::HandleStatus)));
	Routes.Add(Router->BindRoute(FHttpPath(TEXT("/outfit")), EHttpServerRequestVerbs::VERB_POST,
		FHttpRequestHandler::CreateRaw(this, &FThreadSmithBridge::HandleOutfit)));
	Http.StartAllListeners();
	UE_LOG(LogThreadSmith, Log, TEXT("ThreadSmith Bridge listening on port %d"), Settings()->PluginPort);

	// Check now and then whether ThreadSmith is running (for the Wardrobe's status light).
	StatusTicker = FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateRaw(this, &FThreadSmithBridge::PollAppStatus), 3.0f);
}

void FThreadSmithBridge::Stop()
{
	FTSTicker::GetCoreTicker().RemoveTicker(StatusTicker);
	if (Router.IsValid())
	{
		for (const FHttpRouteHandle& H : Routes)
		{
			Router->UnbindRoute(H);
		}
	}
	Routes.Empty();
	Router.Reset();
}

bool FThreadSmithBridge::PollAppStatus(float)
{
	// Just ask Windows whether ThreadSmith is running. (Asking ThreadSmith over
	// HTTP filled the Output Log with time-out warnings whenever it was closed.)
	const FString Exe = FPaths::GetCleanFilename(Settings()->ThreadSmithExe.FilePath);
	bAppConnected = FPlatformProcess::IsApplicationRunning(Exe.IsEmpty() ? TEXT("threadsmith.exe") : *Exe);
	return true; // keep ticking
}

bool FThreadSmithBridge::HandleStatus(const FHttpServerRequest&, const FHttpResultCallback& OnComplete)
{
	TSharedRef<FJsonObject> Obj = MakeShared<FJsonObject>();
	Obj->SetBoolField(TEXT("ok"), true);
	Obj->SetStringField(TEXT("plugin"), TEXT("ThreadSmithBridge"));
	Obj->SetStringField(TEXT("engine"), FEngineVersion::Current().ToString(EVersionComponent::Patch));
	Obj->SetNumberField(TEXT("protocol"), ProtocolVersion);
	OnComplete(JsonResponse(Obj));
	return true;
}

bool FThreadSmithBridge::HandleOutfit(const FHttpServerRequest& Request, const FHttpResultCallback& OnComplete)
{
	TSharedPtr<FJsonObject> Json;
	if (!FJsonSerializer::Deserialize(TJsonReaderFactory<>::Create(BodyAsString(Request)), Json) || !Json.IsValid())
	{
		OnComplete(ErrorResponse(TEXT("Bad JSON")));
		return true;
	}

	// The library folder is the source of truth; fall back to the message fields.
	const FString OutfitDir = Json->GetStringField(TEXT("outfit_dir"));
	TSharedPtr<FThreadSmithOutfit> Outfit = ThreadSmithLibrary::Read(OutfitDir);
	if (!Outfit.IsValid())
	{
		Outfit = MakeShared<FThreadSmithOutfit>();
		Outfit->Dir = FPaths::GetPath(Json->GetStringField(TEXT("mesh_file")));
		Outfit->MeshFile = FPaths::GetCleanFilename(Json->GetStringField(TEXT("mesh_file")));
		Outfit->Name = Json->GetStringField(TEXT("name"));
		Outfit->Id = FMD5::HashAnsiString(*OutfitDir);
	}
	Json->TryGetStringField(TEXT("skeleton_asset"), Outfit->UnrealSkeleton);
	Json->TryGetStringField(TEXT("body_asset"), Outfit->UnrealBody);
	Json->TryGetStringField(TEXT("character"), Outfit->Character);
	const bool bApply = Json->GetBoolField(TEXT("apply_to_character"));

	FString Error;
	USkeletalMesh* Mesh = ImportOutfit(*Outfit, /*bForceReimport*/ true, Error);
	if (!Mesh)
	{
		Notify(FText::Format(LOCTEXT("ImportFailed", "ThreadSmith: couldn't import {0}: {1}"), FText::FromString(Outfit->Name), FText::FromString(Error)), false);
		OnComplete(ErrorResponse(Error, EHttpServerResponseCodes::ServerError));
		return true;
	}

	FString ApplyNote;
	FString AppliedTo;
	USkeletalMesh* TrimmedBody = nullptr;
	if (bApply && !Outfit->TrimmedBodyFile.IsEmpty())
	{
		FString TrimError;
		TrimmedBody = ImportTrimmedBody(*Outfit, TrimError);
		if (!TrimmedBody)
		{
			UE_LOG(LogThreadSmith, Warning, TEXT("Couldn't import the trimmed body: %s"), *TrimError);
		}
	}
	if (bApply)
	{
		// Like MetaTailor: the outfit goes into the character's Blueprint as a
		// component, so every copy wears it and it can be deleted there.
		if (UBlueprint* BP = BlueprintForCharacter(Outfit->Character))
		{
			if (AddOutfitToBlueprint(Mesh, Outfit->Id, BP, Error, TrimmedBody))
			{
				AppliedTo = BP->GetName();
			}
			else
			{
				ApplyNote = Error;
			}
		}
		else if (AActor* Target = FindTargetActor(Outfit->Character))
		{
			// Not a Blueprint character: put it on the placed actor instead.
			if (ApplyOutfitToActor(Mesh, Outfit->Id, Target, Error, TrimmedBody))
			{
				AppliedTo = Target->GetActorLabel();
			}
			else
			{
				ApplyNote = Error;
			}
		}
		else
		{
			ApplyNote = TEXT("no character to put it on: open its Blueprint (or select it) and use Add to character in the Wardrobe");
		}
	}

	FString Done = TEXT("");
	if (!AppliedTo.IsEmpty())
	{
		Done = TEXT(" and added to ") + AppliedTo;
	}
	else if (!ApplyNote.IsEmpty())
	{
		Done = TEXT(" (") + ApplyNote + TEXT(")");
	}
	Notify(FText::Format(LOCTEXT("Imported", "ThreadSmith: {0} imported{1}"), FText::FromString(Outfit->Name), FText::FromString(Done)), ApplyNote.IsEmpty());
	OnLibraryChanged.Broadcast();

	TSharedRef<FJsonObject> Obj = MakeShared<FJsonObject>();
	Obj->SetBoolField(TEXT("ok"), true);
	Obj->SetStringField(TEXT("asset"), Mesh->GetPathName());
	if (!ApplyNote.IsEmpty())
	{
		Obj->SetStringField(TEXT("note"), ApplyNote);
	}
	OnComplete(JsonResponse(Obj));
	return true;
}

// ---------------------------------------------------------------------------
// Unreal -> ThreadSmith
// ---------------------------------------------------------------------------

static bool LooksLikeFace(const USkeletalMeshComponent* C)
{
	const FString Comp = C->GetName();
	const FString Asset = C->GetSkeletalMeshAsset() ? C->GetSkeletalMeshAsset()->GetName() : FString();
	return Comp.Contains(TEXT("Face")) || Asset.Contains(TEXT("Face")) || Asset.Contains(TEXT("Head"));
}

USkeletalMeshComponent* FThreadSmithBridge::FindFaceComponent(AActor* Actor)
{
	if (!Actor)
	{
		return nullptr;
	}
	TArray<USkeletalMeshComponent*> Comps;
	Actor->GetComponents<USkeletalMeshComponent>(Comps);
	for (USkeletalMeshComponent* C : Comps)
	{
		if (C->GetSkeletalMeshAsset() && !C->IsA<UThreadSmithOutfitComponent>() && LooksLikeFace(C))
		{
			return C;
		}
	}
	return nullptr;
}

USkeletalMeshComponent* FThreadSmithBridge::FindBodyComponent(AActor* Actor)
{
	if (!Actor)
	{
		return nullptr;
	}
	// 1. A Character's own mesh (CharacterMesh0) is the body.
	if (const ACharacter* Char = Cast<ACharacter>(Actor))
	{
		USkeletalMeshComponent* Mesh = Char->GetMesh();
		if (Mesh && Mesh->GetSkeletalMeshAsset() && !LooksLikeFace(Mesh))
		{
			return Mesh;
		}
	}
	TArray<USkeletalMeshComponent*> Comps;
	Actor->GetComponents<USkeletalMeshComponent>(Comps);
	// 2. A part named "Body" (MetaHuman blueprints), or whose mesh is a body mesh.
	for (USkeletalMeshComponent* C : Comps)
	{
		if (!C->GetSkeletalMeshAsset() || C->IsA<UThreadSmithOutfitComponent>() || LooksLikeFace(C))
		{
			continue;
		}
		if (C->GetName().Contains(TEXT("Body")) || C->GetSkeletalMeshAsset()->GetName().Contains(TEXT("Body")))
		{
			return C;
		}
	}
	// 3. Otherwise the biggest skeleton that isn't a face or clothes.
	USkeletalMeshComponent* Best = nullptr;
	int32 BestBones = -1;
	for (USkeletalMeshComponent* C : Comps)
	{
		if (!C->GetSkeletalMeshAsset() || C->IsA<UThreadSmithOutfitComponent>() || LooksLikeFace(C))
		{
			continue;
		}
		const int32 Bones = C->GetSkeletalMeshAsset()->GetRefSkeleton().GetNum();
		if (Bones > BestBones)
		{
			Best = C;
			BestBones = Bones;
		}
	}
	return Best;
}

void FThreadSmithBridge::SendSelectionToThreadSmith()
{
	TArray<AActor*> Actors;
	GEditor->GetSelectedActors()->GetSelectedObjects<AActor>(Actors);
	for (AActor* A : Actors)
	{
		if (USkeletalMeshComponent* Body = FindBodyComponent(A))
		{
			USkeletalMeshComponent* Face = FindFaceComponent(A);
			SendBodyToThreadSmith(Body->GetSkeletalMeshAsset(), A, Face ? Face->GetSkeletalMeshAsset() : nullptr,
				Cast<UBlueprint>(A->GetClass()->ClassGeneratedBy));
			return;
		}
	}
	// Nothing selected in the level: use the character Blueprint that's open, if any.
	if (UBlueprint* BP = FindTargetBlueprint())
	{
		SendBlueprintToThreadSmith(BP);
		return;
	}
	Notify(LOCTEXT("NothingSelected", "ThreadSmith: select a MetaHuman (or any character) in the level, or open its Blueprint, first."), false);
}

USkeleton* FThreadSmithBridge::ResolveSkeleton(const FString& SkeletonPath, USkeletalMesh* BodyHint) const
{
	if (!SkeletonPath.IsEmpty())
	{
		if (USkeleton* S = LoadObject<USkeleton>(nullptr, *SkeletonPath))
		{
			return S;
		}
	}
	if (BodyHint && BodyHint->GetSkeleton())
	{
		return BodyHint->GetSkeleton();
	}
	// MetaHuman bodies made in-engine can point at a skeleton inside the
	// MetaHuman Character plugin that isn't loaded; projects keep a copy.
	IAssetRegistry& Registry = FModuleManager::LoadModuleChecked<FAssetRegistryModule>("AssetRegistry").Get();
	TArray<FAssetData> Skeletons;
	Registry.GetAssetsByClass(USkeleton::StaticClass()->GetClassPathName(), Skeletons);
	const FString Wanted = SkeletonPath.IsEmpty() ? TEXT("metahuman_base_skel") : FPaths::GetBaseFilename(SkeletonPath);
	for (const FAssetData& A : Skeletons)
	{
		if (A.AssetName.ToString().Equals(Wanted, ESearchCase::IgnoreCase) && A.PackageName.ToString().StartsWith(TEXT("/Game/")))
		{
			return Cast<USkeleton>(A.GetAsset());
		}
	}
	return nullptr;
}

/** Export a skeletal mesh to FBX (LOD0 only, no morph targets: ThreadSmith doesn't need them). */
static bool ExportMeshFbx(USkeletalMesh* Mesh, const FString& File)
{
	UAssetExportTask* Task = NewObject<UAssetExportTask>();
	Task->Object = Mesh;
	Task->Filename = File;
	Task->bSelected = false;
	Task->bReplaceIdentical = true;
	Task->bPrompt = false;
	Task->bAutomated = true;
	UFbxExportOption* Options = NewObject<UFbxExportOption>();
	Options->bASCII = false;
	Options->LevelOfDetail = false;
	Options->Collision = false;
	Options->bExportMorphTargets = false;
	Task->Options = Options;
	return UExporter::RunAssetExportTask(Task);
}

void FThreadSmithBridge::SendBodyToThreadSmith(USkeletalMesh* BodyMesh, AActor* Actor, USkeletalMesh* HeadMesh, UBlueprint* Blueprint)
{
	if (!BodyMesh)
	{
		return;
	}
	// A body ThreadSmith trimmed under clothes: send the full one, so ThreadSmith gets the whole body.
	BodyMesh = UntrimmedBody(BodyMesh);
	const FString Character = Actor ? Actor->GetActorLabel() : (Blueprint ? Blueprint->GetName() : BodyMesh->GetName());
	const FString Dir = FPaths::ConvertRelativePathToFull(FPaths::ProjectSavedDir() / TEXT("ThreadSmith/Bodies"));
	IFileManager::Get().MakeDirectory(*Dir, true);
	const FString Safe = ObjectTools::SanitizeObjectName(Character);
	const FString File = Dir / (Safe + TEXT("_Body.fbx"));

	if (!ExportMeshFbx(BodyMesh, File))
	{
		Notify(LOCTEXT("ExportFailed", "ThreadSmith: couldn't export the body to FBX."), false);
		return;
	}
	// The head comes too, so collars and hoods can be judged (it isn't fitted to).
	FString HeadFile;
	if (HeadMesh)
	{
		HeadFile = Dir / (Safe + TEXT("_Head.fbx"));
		if (!ExportMeshFbx(HeadMesh, HeadFile))
		{
			UE_LOG(LogThreadSmith, Warning, TEXT("Couldn't export the head %s; sending the body only."), *HeadMesh->GetPathName());
			HeadFile.Reset();
		}
	}

	USkeleton* Skeleton = ResolveSkeleton(FString(), BodyMesh);
	if (Actor)
	{
		SentActors.Add(Character, Actor);
	}
	if (Blueprint)
	{
		SentBlueprints.Add(Character, Blueprint);
	}

	TSharedRef<FJsonObject> Obj = MakeShared<FJsonObject>();
	Obj->SetStringField(TEXT("file"), File);
	Obj->SetStringField(TEXT("character"), Character);
	Obj->SetStringField(TEXT("body_asset"), BodyMesh->GetPathName());
	if (!HeadFile.IsEmpty())
	{
		Obj->SetStringField(TEXT("head_file"), HeadFile);
	}
	if (Skeleton)
	{
		Obj->SetStringField(TEXT("skeleton_asset"), Skeleton->GetPathName());
	}
	else
	{
		UE_LOG(LogThreadSmith, Warning, TEXT("No skeleton found for %s; outfits may import onto a new skeleton."), *BodyMesh->GetPathName());
	}
	Notify(FText::Format(LOCTEXT("Sending", "ThreadSmith: sending {0}…"), FText::FromString(Character)), true);
	bLaunchedApp = false; // start ThreadSmith again if it was closed since last time
	PostBody(ToJson(Obj), /*AttemptsLeft*/ 25);
}

void FThreadSmithBridge::PostBody(const FString& JsonBody, int32 AttemptsLeft)
{
	TSharedRef<IHttpRequest, ESPMode::ThreadSafe> Req = FHttpModule::Get().CreateRequest();
	Req->SetURL(FString::Printf(TEXT("http://127.0.0.1:%d/body"), Settings()->AppPort));
	Req->SetVerb(TEXT("POST"));
	Req->SetHeader(TEXT("Content-Type"), TEXT("application/json"));
	Req->SetContentAsString(JsonBody);
	Req->SetTimeout(5.0f);
	Req->OnProcessRequestComplete().BindLambda([this, JsonBody, AttemptsLeft](FHttpRequestPtr, FHttpResponsePtr Resp, bool bOk)
	{
		if (bOk && Resp.IsValid() && Resp->GetResponseCode() == 200)
		{
			bAppConnected = true;
			Notify(LOCTEXT("Sent", "ThreadSmith: body opened in ThreadSmith."), true);
			return;
		}
		if (bOk && Resp.IsValid())
		{
			Notify(FText::Format(LOCTEXT("Refused", "ThreadSmith refused the body: {0}"), FText::FromString(Resp->GetContentAsString())), false);
			return;
		}
		// Not running: start it once, then keep trying for a while.
		if (!bLaunchedApp && !LaunchThreadSmith())
		{
			Notify(LOCTEXT("NotRunning", "ThreadSmith isn't running. Start it (or set its .exe in Editor Preferences > Plugins > ThreadSmith Bridge) and try again."), false);
			return;
		}
		if (AttemptsLeft <= 0)
		{
			Notify(LOCTEXT("Timeout", "ThreadSmith didn't answer. Is it open?"), false);
			return;
		}
		FTSTicker::GetCoreTicker().AddTicker(FTickerDelegate::CreateLambda([this, JsonBody, AttemptsLeft](float)
		{
			PostBody(JsonBody, AttemptsLeft - 1);
			return false; // run once
		}), 1.5f);
	});
	Req->ProcessRequest();
}

bool FThreadSmithBridge::LaunchThreadSmith()
{
	const FString Exe = Settings()->ThreadSmithExe.FilePath;
	if (Exe.IsEmpty() || !FPaths::FileExists(Exe))
	{
		return false;
	}
	bLaunchedApp = true;
	// "-" = start without a body and wait for one from Unreal.
	FProcHandle Proc = FPlatformProcess::CreateProc(*Exe, TEXT("-"), /*bLaunchDetached*/ true, /*bLaunchHidden*/ false,
		/*bLaunchReallyHidden*/ false, nullptr, 0, *FPaths::GetPath(Exe), nullptr);
	UE_LOG(LogThreadSmith, Log, TEXT("Started ThreadSmith: %s"), *Exe);
	return Proc.IsValid();
}

// ---------------------------------------------------------------------------
// ThreadSmith -> Unreal
// ---------------------------------------------------------------------------

USkeletalMesh* FThreadSmithBridge::ImportOutfit(const FThreadSmithOutfit& Outfit, bool bForceReimport, FString& OutError)
{
	const FString SafeName = ObjectTools::SanitizeObjectName(Outfit.Name.Replace(TEXT(" "), TEXT("_")));
	const FString Folder = Settings()->OutfitContentFolder / (SafeName + TEXT("_") + Outfit.Id.Left(6));
	const FString AssetPath = Folder / SafeName + TEXT(".") + SafeName;

	if (!bForceReimport)
	{
		if (USkeletalMesh* Existing = LoadObject<USkeletalMesh>(nullptr, *AssetPath, nullptr, LOAD_NoWarn | LOAD_Quiet))
		{
			return Existing;
		}
	}

	USkeletalMesh* BodyHint = Outfit.UnrealBody.IsEmpty() ? nullptr : LoadObject<USkeletalMesh>(nullptr, *Outfit.UnrealBody, nullptr, LOAD_NoWarn | LOAD_Quiet);
	USkeleton* Skeleton = ResolveSkeleton(Outfit.UnrealSkeleton, BodyHint);
	if (!Skeleton)
	{
		OutError = TEXT("couldn't find the body skeleton in this project");
		return nullptr;
	}

	return ImportGlbOntoSkeleton(Outfit.MeshPath(), SafeName, Folder, Skeleton, OutError);
}

USkeletalMesh* FThreadSmithBridge::ImportTrimmedBody(const FThreadSmithOutfit& Outfit, FString& OutError)
{
	if (Outfit.TrimmedBodyFile.IsEmpty())
	{
		return nullptr;
	}
	const FString SafeName = ObjectTools::SanitizeObjectName(Outfit.Name.Replace(TEXT(" "), TEXT("_")));
	const FString Folder = Settings()->OutfitContentFolder / (SafeName + TEXT("_") + Outfit.Id.Left(6));
	USkeletalMesh* BodyHint = Outfit.UnrealBody.IsEmpty() ? nullptr : LoadObject<USkeletalMesh>(nullptr, *Outfit.UnrealBody, nullptr, LOAD_NoWarn | LOAD_Quiet);
	USkeleton* Skeleton = ResolveSkeleton(Outfit.UnrealSkeleton, BodyHint);
	if (!Skeleton)
	{
		OutError = TEXT("couldn't find the body skeleton in this project");
		return nullptr;
	}
	return ImportGlbOntoSkeleton(Outfit.TrimmedBodyPath(), SafeName + TEXT("_TrimmedBody"), Folder, Skeleton, OutError);
}

AActor* FThreadSmithBridge::FindTargetActor(const FString& Character) const
{
	if (const TWeakObjectPtr<AActor>* Sent = SentActors.Find(Character))
	{
		if (Sent->IsValid())
		{
			return Sent->Get();
		}
	}
	TArray<AActor*> Actors;
	GEditor->GetSelectedActors()->GetSelectedObjects<AActor>(Actors);
	for (AActor* A : Actors)
	{
		if (FindBodyComponent(A))
		{
			return A;
		}
	}
	return nullptr;
}

bool FThreadSmithBridge::ApplyOutfitToActor(USkeletalMesh* OutfitMesh, const FString& OutfitId, AActor* Actor, FString& OutError, USkeletalMesh* TrimmedBody)
{
	USkeletalMeshComponent* Body = FindBodyComponent(Actor);
	if (!Body)
	{
		OutError = TEXT("that actor has no body mesh");
		return false;
	}

	const FScopedTransaction Transaction(LOCTEXT("ApplyOutfit", "Apply ThreadSmith Outfit"));
	Actor->Modify();

	// Replace the same outfit if it's already on.
	TArray<UThreadSmithOutfitComponent*> Existing;
	Actor->GetComponents<UThreadSmithOutfitComponent>(Existing);
	for (UThreadSmithOutfitComponent* C : Existing)
	{
		if (C->OutfitId == OutfitId)
		{
			C->Modify();
			Actor->RemoveInstanceComponent(C);
			C->DestroyComponent();
		}
	}

	const FName CompName = MakeUniqueObjectName(Actor, UThreadSmithOutfitComponent::StaticClass(),
		FName(*(TEXT("TS_") + OutfitMesh->GetName())));
	UThreadSmithOutfitComponent* Comp = NewObject<UThreadSmithOutfitComponent>(Actor, CompName, RF_Transactional);
	Comp->OutfitId = OutfitId;
	Comp->BodyComponentName = Body->GetFName();
	Comp->SetSkeletalMeshAsset(OutfitMesh);
	Comp->SetupAttachment(Body);
	Actor->AddInstanceComponent(Comp);
	Comp->RegisterComponent();
	Comp->FollowBody();
	if (TrimmedBody)
	{
		// Skin under the clothes removed, so it can't poke through.
		RememberOriginalBody(TrimmedBody, Body->GetSkeletalMeshAsset());
		Body->Modify();
		Body->SetSkeletalMeshAsset(TrimmedBody);
	}
	return true;
}

void FThreadSmithBridge::ApplyOutfitToSelection(const FThreadSmithOutfit& Outfit)
{
	AActor* Target = FindTargetActor(FString());
	if (!Target)
	{
		Notify(LOCTEXT("SelectCharacter", "ThreadSmith: select a character in the level first."), false);
		return;
	}
	FString Error;
	USkeletalMesh* Mesh = ImportOutfit(Outfit, /*bForceReimport*/ false, Error);
	FString TrimError;
	USkeletalMesh* TrimmedBody = Mesh ? ImportTrimmedBody(Outfit, TrimError) : nullptr;
	if (!Mesh || !ApplyOutfitToActor(Mesh, Outfit.Id, Target, Error, TrimmedBody))
	{
		Notify(FText::Format(LOCTEXT("ApplyFailed", "ThreadSmith: couldn't apply {0}: {1}"), FText::FromString(Outfit.Name), FText::FromString(Error)), false);
		return;
	}
	Notify(FText::Format(LOCTEXT("Applied", "ThreadSmith: {0} put on {1}"), FText::FromString(Outfit.Name), FText::FromString(Target->GetActorLabel())), true);
}

// ---------------------------------------------------------------------------
// Adding outfits to a character Blueprint
// ---------------------------------------------------------------------------

static bool IsBodyCandidate(const USkeletalMeshComponent* C)
{
	return C && !C->IsA<UThreadSmithOutfitComponent>() && C->GetSkeletalMeshAsset() && !LooksLikeFace(C);
}

/** The body inside a Blueprint: either a native component (a Character's CharacterMesh0) or a node the Blueprint added (MetaHuman "Body"). */
struct FBlueprintBody
{
	USkeletalMeshComponent* NativeComponent = nullptr; // from the class default object
	USCS_Node* Node = nullptr;                          // added in this Blueprint
	FName Name;
};

static FBlueprintBody FindBlueprintBody(UBlueprint* Blueprint)
{
	FBlueprintBody Result;
	if (!Blueprint || !Blueprint->GeneratedClass)
	{
		return Result;
	}
	// 1. A Character's own mesh.
	if (ACharacter* CDO = Cast<ACharacter>(Blueprint->GeneratedClass->GetDefaultObject()))
	{
		if (IsBodyCandidate(CDO->GetMesh()))
		{
			Result.NativeComponent = CDO->GetMesh();
			Result.Name = CDO->GetMesh()->GetFName();
			return Result;
		}
	}
	// 2. A skeletal mesh node this Blueprint added: prefer one called Body.
	if (USimpleConstructionScript* SCS = Blueprint->SimpleConstructionScript)
	{
		USCS_Node* Best = nullptr;
		for (USCS_Node* Node : SCS->GetAllNodes())
		{
			USkeletalMeshComponent* T = Cast<USkeletalMeshComponent>(Node->ComponentTemplate);
			if (!IsBodyCandidate(T))
			{
				continue;
			}
			const bool bBody = Node->GetVariableName().ToString().Contains(TEXT("Body"))
				|| T->GetSkeletalMeshAsset()->GetName().Contains(TEXT("Body"));
			if (bBody || !Best)
			{
				Best = Node;
				if (bBody)
				{
					break;
				}
			}
		}
		if (Best)
		{
			Result.Node = Best;
			Result.Name = Best->GetVariableName();
			return Result;
		}
	}
	// 3. Any other native skeletal mesh on the class default object.
	if (AActor* CDO = Cast<AActor>(Blueprint->GeneratedClass->GetDefaultObject()))
	{
		TArray<USkeletalMeshComponent*> Comps;
		CDO->GetComponents<USkeletalMeshComponent>(Comps);
		for (USkeletalMeshComponent* C : Comps)
		{
			if (IsBodyCandidate(C))
			{
				Result.NativeComponent = C;
				Result.Name = C->GetFName();
				break;
			}
		}
	}
	return Result;
}

UBlueprint* FThreadSmithBridge::FindTargetBlueprint() const
{
	// 1. A Blueprint open in its editor that has a body (like MetaTailor's docked panel).
	if (GEditor)
	{
		if (UAssetEditorSubsystem* Editors = GEditor->GetEditorSubsystem<UAssetEditorSubsystem>())
		{
			UBlueprint* Found = nullptr;
			double Newest = -1.0;
			for (UObject* Asset : Editors->GetAllEditedAssets())
			{
				UBlueprint* BP = Cast<UBlueprint>(Asset);
				if (!BP || !BP->GeneratedClass || !BP->GeneratedClass->IsChildOf(AActor::StaticClass()))
				{
					continue;
				}
				const FBlueprintBody Body = FindBlueprintBody(BP);
				if (!Body.NativeComponent && !Body.Node)
				{
					continue;
				}
				// The editor the user looked at last.
				IAssetEditorInstance* Instance = Editors->FindEditorForAsset(BP, /*bFocusIfOpen*/ false);
				const double Time = Instance ? Instance->GetLastActivationTime() : 0.0;
				if (Time > Newest)
				{
					Newest = Time;
					Found = BP;
				}
			}
			if (Found)
			{
				return Found;
			}
		}
	}
	// 2. The Blueprint of the character selected in the level.
	if (AActor* Actor = FindTargetActor(FString()))
	{
		return Cast<UBlueprint>(Actor->GetClass()->ClassGeneratedBy);
	}
	return nullptr;
}

bool FThreadSmithBridge::AddOutfitToBlueprint(USkeletalMesh* OutfitMesh, const FString& OutfitId, UBlueprint* Blueprint, FString& OutError, USkeletalMesh* TrimmedBody)
{
	if (!Blueprint || !Blueprint->SimpleConstructionScript)
	{
		OutError = TEXT("no character Blueprint");
		return false;
	}
	const FBlueprintBody Body = FindBlueprintBody(Blueprint);
	if (!Body.NativeComponent && !Body.Node)
	{
		OutError = TEXT("that Blueprint has no body mesh");
		return false;
	}

	const FScopedTransaction Transaction(LOCTEXT("AddOutfitToBP", "Add ThreadSmith Outfit to Blueprint"));
	Blueprint->Modify();
	USimpleConstructionScript* SCS = Blueprint->SimpleConstructionScript;
	SCS->Modify();

	// Replace the same outfit if it's already there (same library id, or a newer
	// save of the same outfit: same mesh name), instead of stacking copies.
	auto IsSameOutfit = [&](const UThreadSmithOutfitComponent* T)
	{
		return T && (T->OutfitId == OutfitId
			|| (T->GetSkeletalMeshAsset() && T->GetSkeletalMeshAsset()->GetName() == OutfitMesh->GetName()));
	};
	TArray<USCS_Node*> ToRemove;
	for (USCS_Node* Node : SCS->GetAllNodes())
	{
		if (IsSameOutfit(Cast<UThreadSmithOutfitComponent>(Node->ComponentTemplate)))
		{
			ToRemove.Add(Node);
		}
	}
	for (USCS_Node* Node : ToRemove)
	{
		SCS->RemoveNodeAndPromoteChildren(Node);
	}

	// Older versions of the plugin put outfits on placed characters only.
	// Take those copies off, since the Blueprint now has it.
	if (GEditor && Blueprint->GeneratedClass && Blueprint->GeneratedClass->IsChildOf(AActor::StaticClass()))
	{
		if (UWorld* World = GEditor->GetEditorWorldContext().World())
		{
			const TSubclassOf<AActor> ActorClass(Blueprint->GeneratedClass.Get());
			for (TActorIterator<AActor> It(World, ActorClass); It; ++It)
			{
				TArray<UThreadSmithOutfitComponent*> Comps;
				It->GetComponents<UThreadSmithOutfitComponent>(Comps);
				for (UThreadSmithOutfitComponent* C : Comps)
				{
					if (C->CreationMethod == EComponentCreationMethod::Instance && IsSameOutfit(C))
					{
						It->Modify();
						C->Modify();
						It->RemoveInstanceComponent(C);
						C->DestroyComponent();
					}
				}
			}
		}
	}

	const FName VarName = FBlueprintEditorUtils::FindUniqueKismetName(Blueprint, TEXT("TS_") + OutfitMesh->GetName());
	USCS_Node* NewNode = SCS->CreateNode(UThreadSmithOutfitComponent::StaticClass(), VarName);
	UThreadSmithOutfitComponent* Template = CastChecked<UThreadSmithOutfitComponent>(NewNode->ComponentTemplate);
	Template->SetSkeletalMeshAsset(OutfitMesh);
	Template->OutfitId = OutfitId;
	Template->BodyComponentName = Body.Name;

	if (Body.Node)
	{
		Body.Node->AddChildNode(NewNode);
	}
	else
	{
		// Parent to an inherited (C++) component, the same way MetaTailor does.
		SCS->AddNode(NewNode);
		NewNode->SetParent(Body.NativeComponent);
	}

	// The body with the skin under the clothes removed (if ThreadSmith made one).
	if (TrimmedBody)
	{
		if (Body.Node)
		{
			if (USkeletalMeshComponent* BodyTemplate = Cast<USkeletalMeshComponent>(Body.Node->ComponentTemplate))
			{
				RememberOriginalBody(TrimmedBody, BodyTemplate->GetSkeletalMeshAsset());
				BodyTemplate->Modify();
				BodyTemplate->SetSkeletalMeshAsset(TrimmedBody);
			}
		}
		else
		{
			UE_LOG(LogThreadSmith, Warning, TEXT("%s's body comes from C++, so the trimmed body (%s) wasn't swapped in. Set it on the mesh component by hand."),
				*Blueprint->GetName(), *TrimmedBody->GetPathName());
		}
	}

	FBlueprintEditorUtils::MarkBlueprintAsStructurallyModified(Blueprint);
	FKismetEditorUtilities::CompileBlueprint(Blueprint);
	// Save it, so the outfit is still there next time (the imported mesh is saved too).
	UEditorLoadingAndSavingUtils::SavePackages({ Blueprint->GetPackage() }, /*bOnlyDirty*/ true);
	return true;
}

void FThreadSmithBridge::AddOutfitToTargetBlueprint(const FThreadSmithOutfit& Outfit)
{
	UBlueprint* BP = FindTargetBlueprint();
	if (!BP)
	{
		// A placed character that isn't a Blueprint: dress that one actor.
		if (FindTargetActor(FString()))
		{
			ApplyOutfitToSelection(Outfit);
			return;
		}
		Notify(LOCTEXT("OpenBlueprint", "ThreadSmith: open your character's Blueprint (or select the character in the level) first."), false);
		return;
	}
	FString Error;
	USkeletalMesh* Mesh = ImportOutfit(Outfit, /*bForceReimport*/ false, Error);
	FString TrimError;
	USkeletalMesh* TrimmedBody = Mesh ? ImportTrimmedBody(Outfit, TrimError) : nullptr;
	if (!Mesh || !AddOutfitToBlueprint(Mesh, Outfit.Id, BP, Error, TrimmedBody))
	{
		Notify(FText::Format(LOCTEXT("AddBPFailed", "ThreadSmith: couldn't add {0} to {1}: {2}"), FText::FromString(Outfit.Name),
			FText::FromString(BP->GetName()), FText::FromString(Error)), false);
		return;
	}
	Notify(FText::Format(LOCTEXT("AddedBP", "ThreadSmith: {0} added to {1}."), FText::FromString(Outfit.Name),
		FText::FromString(BP->GetName())), true);
}

UBlueprint* FThreadSmithBridge::BlueprintForCharacter(const FString& Character) const
{
	if (const TWeakObjectPtr<UBlueprint>* Sent = SentBlueprints.Find(Character))
	{
		if (Sent->IsValid())
		{
			return Sent->Get();
		}
	}
	if (const TWeakObjectPtr<AActor>* Actor = SentActors.Find(Character))
	{
		if (Actor->IsValid())
		{
			if (UBlueprint* BP = Cast<UBlueprint>(Actor->Get()->GetClass()->ClassGeneratedBy))
			{
				return BP;
			}
		}
	}
	return FindTargetBlueprint();
}

/** The head / face mesh in a Blueprint (a native component or one the Blueprint added). */
static USkeletalMesh* FindBlueprintFace(UBlueprint* Blueprint)
{
	if (!Blueprint || !Blueprint->GeneratedClass)
	{
		return nullptr;
	}
	if (USimpleConstructionScript* SCS = Blueprint->SimpleConstructionScript)
	{
		for (USCS_Node* Node : SCS->GetAllNodes())
		{
			const USkeletalMeshComponent* T = Cast<USkeletalMeshComponent>(Node->ComponentTemplate);
			if (T && T->GetSkeletalMeshAsset() && !T->IsA<UThreadSmithOutfitComponent>()
				&& (LooksLikeFace(T) || Node->GetVariableName().ToString().Contains(TEXT("Face"))))
			{
				return T->GetSkeletalMeshAsset();
			}
		}
	}
	if (AActor* CDO = Cast<AActor>(Blueprint->GeneratedClass->GetDefaultObject()))
	{
		TArray<USkeletalMeshComponent*> Comps;
		CDO->GetComponents<USkeletalMeshComponent>(Comps);
		for (USkeletalMeshComponent* C : Comps)
		{
			if (C->GetSkeletalMeshAsset() && LooksLikeFace(C))
			{
				return C->GetSkeletalMeshAsset();
			}
		}
	}
	return nullptr;
}

void FThreadSmithBridge::SendBlueprintToThreadSmith(UBlueprint* Blueprint)
{
	const FBlueprintBody Body = FindBlueprintBody(Blueprint);
	USkeletalMesh* BodyMesh = nullptr;
	if (Body.NativeComponent)
	{
		BodyMesh = Body.NativeComponent->GetSkeletalMeshAsset();
	}
	else if (Body.Node)
	{
		if (const USkeletalMeshComponent* T = Cast<USkeletalMeshComponent>(Body.Node->ComponentTemplate))
		{
			BodyMesh = T->GetSkeletalMeshAsset();
		}
	}
	if (!BodyMesh)
	{
		Notify(LOCTEXT("NoBodyInBP", "ThreadSmith: this Blueprint has no body mesh to send."), false);
		return;
	}
	SendBodyToThreadSmith(BodyMesh, nullptr, FindBlueprintFace(Blueprint), Blueprint);
}

#undef LOCTEXT_NAMESPACE
