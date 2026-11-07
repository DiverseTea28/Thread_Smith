#include "ThreadSmithLibrary.h"

#include "Dom/JsonObject.h"
#include "HAL/FileManager.h"
#include "Misc/FileHelper.h"
#include "Misc/Paths.h"
#include "Serialization/JsonReader.h"
#include "Serialization/JsonSerializer.h"

TSharedPtr<FThreadSmithOutfit> ThreadSmithLibrary::Read(const FString& OutfitDir)
{
	FString Text;
	if (!FFileHelper::LoadFileToString(Text, *FPaths::Combine(OutfitDir, TEXT("outfit.json"))))
	{
		return nullptr;
	}
	TSharedPtr<FJsonObject> Json;
	if (!FJsonSerializer::Deserialize(TJsonReaderFactory<>::Create(Text), Json) || !Json.IsValid())
	{
		return nullptr;
	}
	if (Json->GetIntegerField(TEXT("format")) > FormatVersion)
	{
		return nullptr; // made by a newer ThreadSmith
	}

	TSharedPtr<FThreadSmithOutfit> O = MakeShared<FThreadSmithOutfit>();
	O->Dir = OutfitDir;
	O->Id = Json->GetStringField(TEXT("id"));
	O->Name = Json->GetStringField(TEXT("name"));
	O->Category = Json->GetStringField(TEXT("category"));
	O->MeshFile = Json->GetStringField(TEXT("mesh_file"));
	O->ThumbnailFile = Json->GetStringField(TEXT("thumbnail_file"));
	O->Modified = (int64)Json->GetNumberField(TEXT("modified"));

	const TSharedPtr<FJsonObject>* Body = nullptr;
	if (Json->TryGetObjectField(TEXT("body"), Body) && Body)
	{
		(*Body)->TryGetStringField(TEXT("character"), O->Character);
		(*Body)->TryGetStringField(TEXT("unreal_skeleton"), O->UnrealSkeleton);
		(*Body)->TryGetStringField(TEXT("unreal_body"), O->UnrealBody);
	}
	const TArray<TSharedPtr<FJsonValue>>* Garments = nullptr;
	if (Json->TryGetArrayField(TEXT("garments"), Garments) && Garments)
	{
		for (const TSharedPtr<FJsonValue>& G : *Garments)
		{
			if (const TSharedPtr<FJsonObject>* GO = nullptr; G->TryGetObject(GO) && GO)
			{
				O->Triangles += (int32)(*GO)->GetNumberField(TEXT("triangles"));
			}
		}
	}
	if (O->Name.IsEmpty() || !FPaths::FileExists(O->MeshPath()))
	{
		return nullptr;
	}
	return O;
}

TArray<TSharedPtr<FThreadSmithOutfit>> ThreadSmithLibrary::List(const FString& LibraryFolder)
{
	TArray<TSharedPtr<FThreadSmithOutfit>> Out;
	const FString OutfitsDir = FPaths::Combine(LibraryFolder, TEXT("Outfits"));
	TArray<FString> Dirs;
	IFileManager::Get().FindFiles(Dirs, *FPaths::Combine(OutfitsDir, TEXT("*")), /*Files*/ false, /*Directories*/ true);
	for (const FString& D : Dirs)
	{
		if (TSharedPtr<FThreadSmithOutfit> O = Read(FPaths::Combine(OutfitsDir, D)))
		{
			Out.Add(O);
		}
	}
	Out.Sort([](const TSharedPtr<FThreadSmithOutfit>& A, const TSharedPtr<FThreadSmithOutfit>& B) { return A->Modified > B->Modified; });
	return Out;
}
