import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
export function MeetingTags({ meetingId, onChanged }: { meetingId: number; onChanged: () => void }) {
    const [tags,setTags]=useState<string[]>([]);
    const [draft,setDraft]=useState("");
    const [editing,setEditing]=useState(false);
    const [loading,setLoading]=useState(true);
    const [busy,setBusy]=useState(false);
    const [error,setError]=useState("");
    const pending=useRef(false);
    const alive=useRef(true);
    useEffect(()=>{
        alive.current=true;
        void invoke<string[]>("get_meeting_tags",{meetingId}).then(list=>{if(alive.current){setTags(list);setDraft(list.join(", "));}})
            .catch(err=>{if(alive.current)setError(String(err));}).finally(()=>{if(alive.current)setLoading(false);});
        return ()=>{alive.current=false;};
    },[meetingId]);
    const save=async()=>{
        if(pending.current)return;
        pending.current=true;setBusy(true);setError("");
        try{const list=await invoke<string[]>("set_meeting_tags",{meetingId,tags:draft.split(",")});
            if(alive.current){setTags(list);setDraft(list.join(", "));setEditing(false);onChanged();}}
        catch(err){if(alive.current)setError(String(err));}
        finally{pending.current=false;if(alive.current)setBusy(false);}
    };
    return <div className="meeting-tags">
        <span>Tags</span>{tags.map(tag=><span key={tag} className="meeting-tag-chip">{tag}</span>)}
        {!editing && <button type="button" className="turn-clip-btn" disabled={loading} onClick={()=>setEditing(true)}>{tags.length?"Edit tags":"Add tags"}</button>}
        {editing && <><input aria-label="Meeting tags, separated by commas" placeholder="Project, Client, Planning" value={draft} disabled={busy} onChange={event=>setDraft(event.target.value)} />
            <button type="button" className="turn-clip-btn" disabled={busy} onClick={save}>{busy?"Saving…":"Save tags"}</button>
            <button type="button" className="turn-clip-btn" disabled={busy} onClick={()=>{setDraft(tags.join(", "));setEditing(false);}}>Cancel</button></>}
        {error&&<p role="alert">{error}</p>}
    </div>;
}
