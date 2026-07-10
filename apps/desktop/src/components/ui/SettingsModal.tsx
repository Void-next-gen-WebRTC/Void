import { X, User, Mic, Gamepad2, RefreshCw, FlaskConical } from 'lucide-react';
import { useState, useEffect } from 'react';
import { createPortal } from 'react-dom';
import { SettingsModalProps } from '../../models/ui/settingsModalProps.model';
import { ProfileSettings } from '../settings/ProfileSettings';
import { VoiceVideoSettings } from '../settings/VoiceVideoSettings';
import { ActivitySettings } from '../settings/ActivitySettings';
import { UpdateSettings } from '../settings/UpdateSettings';
import { ExperimentalSettings } from '../settings/ExperimentalSettings';

export const SettingsModal = ({ isOpen, onClose, updateAvailable, updateStatus, triggerUpdate, checkForUpdate }: SettingsModalProps) => {
    const [activeTab, setActiveTab] = useState<'profile' | 'voice' | 'activity' | 'update' | 'experimental'>('profile');
    const [mounted, setMounted] = useState(false);

    useEffect(() => {
        setMounted(true);
        
        const handleEscape = (e: KeyboardEvent) => {
            if (e.key === 'Escape') onClose();
        };
        
        if (isOpen) {
            window.addEventListener('keydown', handleEscape);
        }
        return () => window.removeEventListener('keydown', handleEscape);
    }, [isOpen, onClose]);

    if (!isOpen || !mounted) return null;

    const modalContent = (
        <div className="fixed inset-0 z-[9999] flex items-center justify-center bg-[#000000]/60 backdrop-blur-xl animate-in fade-in duration-200">
            <div className="flex w-[800px] h-[600px] glass-modal rounded-2xl overflow-hidden shadow-2xl relative">
                {/* Top edge highlight */}
                <div className="absolute top-0 left-0 right-0 h-px bg-gradient-to-r from-transparent via-white/10 to-transparent z-20 pointer-events-none" />
                {/* Left edge highlight */}
                <div className="absolute top-0 left-0 bottom-0 w-px bg-gradient-to-b from-white/10 via-transparent to-transparent z-20 pointer-events-none" />

                {/* Sidebar */}
                <div className="w-[240px] glass-heavy flex flex-col py-6 px-4 shrink-0 relative z-10 border-r border-white/[0.06]">
                    <div className="absolute top-0 bottom-0 right-0 w-px bg-gradient-to-b from-transparent via-cyan-500/10 to-transparent pointer-events-none" />
                    <nav className="w-full flex flex-col gap-1">
                        <div className="px-2 pb-2 text-[10px] font-black text-cyan-500 uppercase tracking-widest">Paramètres Utilisateur</div>
                        
                        <button 
                            className={`flex items-center gap-3 px-3 py-2 rounded font-bold text-[13px] transition-all duration-200 ${
                                activeTab === 'profile' 
                                ? 'text-white bg-white/[0.08]' 
                                : 'text-gray-400 hover:bg-white/[0.04] hover:text-gray-200'
                            }`}
                            onClick={() => setActiveTab('profile')}
                        >
                            <User size={16} /> Mon Profil
                        </button>
                        
                        <button 
                            className={`flex items-center gap-3 px-3 py-2 rounded font-bold text-[13px] transition-all duration-200 ${
                                activeTab === 'voice' 
                                ? 'text-white bg-white/[0.08]' 
                                : 'text-gray-400 hover:bg-white/[0.04] hover:text-gray-200'
                            }`}
                            onClick={() => setActiveTab('voice')}
                        >
                            <Mic size={16} />
                            Voix & Vidéo
                        </button>
                    </nav>
                    
                    <div className="my-4 h-px bg-white/5 mx-2" />
                    
                    <nav className="w-full flex flex-col gap-1">
                        <div className="px-2 pb-2 text-[10px] font-black text-cyan-500 uppercase tracking-widest">Paramètres d'activité</div>
                        <button 
                            className={`flex items-center gap-3 px-3 py-2 rounded font-bold text-[13px] transition-all duration-200 ${
                                activeTab === 'activity' 
                                ? 'text-white bg-white/[0.08]' 
                                : 'text-gray-400 hover:bg-white/[0.04] hover:text-gray-200'
                            }`}
                            onClick={() => setActiveTab('activity')}
                        >
                            <Gamepad2 size={16} /> Jeux enregistrés
                        </button>
                    </nav>
                    
                    <div className="my-4 h-px bg-white/5 mx-2" />
                    
                    <nav className="w-full flex flex-col gap-1">
                        <div className="px-2 pb-2 text-[10px] font-black text-cyan-500 uppercase tracking-widest">Système</div>
                        <button 
                            className={`flex items-center justify-between px-3 py-2 rounded font-bold text-[13px] transition-all duration-200 ${
                                activeTab === 'update' 
                                ? 'text-white bg-white/[0.08]' 
                                : 'text-gray-400 hover:bg-white/[0.04] hover:text-gray-200'
                            }`}
                            onClick={() => setActiveTab('update')}
                        >
                            <RefreshCw size={16} /> Mises à jour
                            {updateAvailable && <span className="w-2.5 h-2.5 bg-red-400 rounded-full" />}
                        </button>
                    </nav>

                    <div className="my-4 h-px bg-white/5 mx-2" />

                    <nav className="w-full flex flex-col gap-1">
                        <div className="px-2 pb-2 text-[10px] font-black text-amber-500 uppercase tracking-widest">Avancé</div>
                        <button
                            className={`flex items-center gap-3 px-3 py-2 rounded font-bold text-[13px] transition-all duration-200 ${
                                activeTab === 'experimental'
                                ? 'text-white bg-white/[0.08]'
                                : 'text-gray-400 hover:bg-white/[0.04] hover:text-gray-200'
                            }`}
                            onClick={() => setActiveTab('experimental')}
                        >
                            <FlaskConical size={16} />
                            Expérimental
                        </button>
                    </nav>
                </div>
                
                {/* Main Content Area */}
                <div className="flex-1 flex flex-col relative glass-light z-10 w-full">
                    <div className="absolute top-0 left-0 right-0 h-px bg-gradient-to-r from-transparent via-white/[0.06] to-transparent pointer-events-none" />
                    <div className="absolute top-6 right-3 z-50">
                        <button
                            className="w-8 h-8 rounded-full border border-[#1a3a4c] text-gray-400 flex items-center justify-center hover:text-white hover:border-cyan-400 transition-colors"
                            onClick={onClose}
                        >
                            <X size={16} />
                        </button>
                    </div>
                    
                    <div className="flex-1 overflow-y-auto px-10 py-8 custom-scrollbar relative">
                        <div className="max-w-[500px] h-full">
                            {activeTab === 'profile' && <ProfileSettings />}
                            {activeTab === 'voice' && <VoiceVideoSettings />}
                            {activeTab === 'activity' && <ActivitySettings />}
                            {activeTab === 'update' && (
                                <UpdateSettings 
                                    updateAvailable={updateAvailable}
                                    updateStatus={updateStatus}
                                    triggerUpdate={triggerUpdate}
                                    checkForUpdate={checkForUpdate}
                                />
                            )}
                            {activeTab === 'experimental' && <ExperimentalSettings />}
                        </div>
                    </div>
                </div>
            </div>
        </div>
    );

    return createPortal(modalContent, document.body);
};

export default SettingsModal;
